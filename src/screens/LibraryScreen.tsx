import React, {useState, useEffect, useRef} from 'react';
import {View, Text, FlatList, TouchableOpacity, RefreshControl, Image, Alert, ActivityIndicator, Platform, PermissionsAndroid, TextInput, Modal, ScrollView} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {useFocusEffect} from '@react-navigation/native';
import {Ionicons} from '@expo/vector-icons';
import {useStyles} from '../hooks/useStyles';
import {useTheme} from '../styles/theme';
import type {Theme} from '../hooks/useStyles';
import {
    getBooksWithFilters,
    syncPodcastEpisodes,
    getAllSeries,
    getAllCategories,
    getAllAccounts,
    initializeDatabase,
    refreshToken,
    enqueueDownloadNew,
    listDownloadTasks,
    getStageProgress,
    pauseDownload,
    resumeDownload,
    cancelDownload,
    stopDownloadMonitoring,
    retryConversion,
    getBookFilePath,
    clearBookDownloadState,
    setBookFilePath,
    createCoverArtFile,
    requestNotificationPermission,
    getPrimaryAccount,
    getAccount,
    saveAccount,
    downloadLibrivoxFile,
    copyTextToClipboard,
} from '../../modules/expo-rust-bridge';
import type {Book, Account, DownloadTask, StageProgress} from '../../modules/expo-rust-bridge';
import * as SecureStore from 'expo-secure-store';
import * as DocumentPicker from 'expo-document-picker';
import {Directory, Paths} from 'expo-file-system';
import {getDatabasePath} from '../utils/appPaths';
import {getBook} from '../services/librivox';
import {DEMO_BOOKS, DEMO_SERIES, DEMO_CATEGORIES, DEMO_ACCOUNT} from '../services/demo/demoData';
import {isDemoAccountId, isDemoBook, filterSortPaginate} from '../services/demo/demoMode';
import * as demoDownloads from '../services/demo/demoDownloads';
import {
    buildLibraryExportText,
    exportLibrary,
    type LibraryExportDirection,
    type LibraryExportFormat,
    type LibraryExportSortField,
} from '../utils/libraryExport';

const DOWNLOAD_PATH_KEY = 'download_path';
const SELECTED_ACCOUNT_KEY = 'selected_audible_account_id';
const LIBRARY_PREFS_KEY = 'library_preferences';
const INCLUDE_PODCASTS_KEY = 'include_podcasts';
const PAGE_SIZE = 100;

type SortField = 'title' | 'release_date' | 'date_added' | 'series' | 'length' | 'downloaded';
type BookSortField = Exclude<SortField, 'downloaded'>;
type SortDirection = 'asc' | 'desc';
type SourceFilter = 'all' | 'audible' | 'librivox';
type IoniconName = React.ComponentProps<typeof Ionicons>['name'];

const EXPORT_FORMAT_OPTIONS: Array<{ format: LibraryExportFormat; label: string; icon: IoniconName }> = [
    {format: 'csv', label: 'CSV', icon: 'grid-outline'},
    {format: 'txt', label: 'TXT', icon: 'list-outline'},
    {format: 'json', label: 'JSON', icon: 'code-slash-outline'},
    {format: 'xlsx', label: 'XLSX', icon: 'document-text-outline'},
    {format: 'png', label: 'PNG', icon: 'image-outline'},
    {format: 'goodreads', label: 'Goodreads', icon: 'library-outline'},
];

interface LibraryPreferences {
    sortField: SortField;
    sortDirection: SortDirection;
    downloadedGroupSortField?: BookSortField;
    downloadedGroupSortDirection?: SortDirection;
    sourceFilter?: SourceFilter;
}

export default function LibraryScreen() {
    const styles = useStyles(createStyles);
    const { colors } = useTheme();

    // Book data
    const [audiobooks, setAudiobooks] = useState<Book[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [totalCount, setTotalCount] = useState(0);
    const [hasMore, setHasMore] = useState(true);

    // Download tracking
    const [downloadTasks, setDownloadTasks] = useState<Map<string, DownloadTask>>(new Map());
    const [stageProgress, setStageProgress] = useState<Record<string, StageProgress>>({});
    const [downloadSpeeds, setDownloadSpeeds] = useState<Record<string, number>>({}); // bytes/sec
    const prevBytesRef = useRef<Map<string, {bytes: number; time: number}>>(new Map());
    const progressInterval = useRef<NodeJS.Timeout | null>(null);

    // Search, filter, and sort state
    const [searchQuery, setSearchQuery] = useState('');
    const [sortField, setSortField] = useState<SortField>('title');
    const [sortDirection, setSortDirection] = useState<SortDirection>('asc');
    const [downloadedGroupSortField, setDownloadedGroupSortField] = useState<BookSortField>('title');
    const [downloadedGroupSortDirection, setDownloadedGroupSortDirection] = useState<SortDirection>('asc');
    const [selectedSeries, setSelectedSeries] = useState<string[]>([]);
    const [selectedCategories, setSelectedCategories] = useState<string[]>([]);
    const [sourceFilter, setSourceFilter] = useState<SourceFilter>('all');
    const [typeFilter, setTypeFilter] = useState<'all' | 'audiobooks' | 'podcasts'>('all');
    const [accountFilters, setAccountFilters] = useState<string[]>([]);
    const [expandedFilterSections, setExpandedFilterSections] = useState<Record<string, boolean>>({});
    // Downloads need one owning account; only use the filter when it's unambiguous
    const singleAccountFilter = accountFilters.length === 1 ? accountFilters[0] : null;
    const [includePodcasts, setIncludePodcasts] = useState(true);

    // Filter options
    const [allSeries, setAllSeries] = useState<string[]>([]);
    const [allCategories, setAllCategories] = useState<string[]>([]);
    const [allAccounts, setAllAccounts] = useState<Account[]>([]);

    // Modal state
    const [showFilterModal, setShowFilterModal] = useState(false);
    const [showSortModal, setShowSortModal] = useState(false);
    const [showContextMenu, setShowContextMenu] = useState(false);
    const [showDetailModal, setShowDetailModal] = useState(false);
    const [detailDescriptionExpanded, setDetailDescriptionExpanded] = useState(false);
    // Measured line count of the full (unclamped) description; drives whether
    // the Show more/less toggle is needed at all.
    const [detailDescriptionLines, setDetailDescriptionLines] = useState(0);
    const DETAIL_DESCRIPTION_LINES = 8;
    const [selectedBook, setSelectedBook] = useState<Book | null>(null);
    const [selectedPodcast, setSelectedPodcast] = useState<Book | null>(null);
    const [podcastEpisodes, setPodcastEpisodes] = useState<Book[]>([]);
    const [podcastEpisodeCount, setPodcastEpisodeCount] = useState(0);
    const [isLoadingPodcastEpisodes, setIsLoadingPodcastEpisodes] = useState(false);
    const [isRefreshingPodcastEpisodes, setIsRefreshingPodcastEpisodes] = useState(false);
    const [isLoadingMorePodcastEpisodes, setIsLoadingMorePodcastEpisodes] = useState(false);
    const [hasMorePodcastEpisodes, setHasMorePodcastEpisodes] = useState(false);

    // Controls visibility
    const [showControls, setShowControls] = useState(false);
    const [showExportControls, setShowExportControls] = useState(false);

    // Batch download: multi-select mode + the set of selected ASINs.
    const [batchMode, setBatchMode] = useState(false);
    const [selectedForBatch, setSelectedForBatch] = useState<Set<string>>(new Set());

    // Export state
    const [exportFormats, setExportFormats] = useState<LibraryExportFormat[]>(['csv', 'txt', 'json', 'xlsx', 'png']);
    const [exportSortField, setExportSortField] = useState<LibraryExportSortField>('title');
    const [exportSortDirection, setExportSortDirection] = useState<LibraryExportDirection>('asc');
    const [exportGroupByAuthor, setExportGroupByAuthor] = useState(false);
    const [exportGroupBySeries, setExportGroupBySeries] = useState(false);
    const [isExporting, setIsExporting] = useState(false);
    const [isCopyingExportText, setIsCopyingExportText] = useState(false);

    // Debounce search
    const searchTimeout = useRef<NodeJS.Timeout | null>(null);

    // Load saved preferences on mount
    useEffect(() => {
        loadPreferences();
        loadFilterOptions();
    }, []);

    // Load books when filters change
    useEffect(() => {
        // Debounce search
        if (searchTimeout.current) {
            clearTimeout(searchTimeout.current);
        }

        searchTimeout.current = setTimeout(() => {
            loadBooks(true);
        }, 300);

        return () => {
            if (searchTimeout.current) {
                clearTimeout(searchTimeout.current);
            }
        };
    }, [searchQuery, sortField, sortDirection, downloadedGroupSortField, downloadedGroupSortDirection, selectedSeries, selectedCategories, sourceFilter, typeFilter, accountFilters, includePodcasts]);

    // Reload books when tab is focused
    useFocusEffect(
        React.useCallback(() => {
            console.log('[LibraryScreen] Tab focused, reloading books...');
            loadBooks(true);
        }, [searchQuery, sortField, sortDirection, downloadedGroupSortField, downloadedGroupSortDirection, selectedSeries, selectedCategories, sourceFilter, typeFilter, accountFilters, includePodcasts])
    );

    useFocusEffect(
        React.useCallback(() => {
            SecureStore.getItemAsync(INCLUDE_PODCASTS_KEY).then(value => {
                setIncludePodcasts(value !== 'false');
            });
            loadFilterOptions();
        }, [])
    );

    // Poll for download progress
    useEffect(() => {
        const pollProgress = () => {
            try {
                const dbPath = getDatabasePath();

                const tasks = listDownloadTasks(dbPath);
                const taskMap = new Map<string, DownloadTask>();

                tasks.forEach(task => {
                    taskMap.set(task.asin, task);
                });

                // Merge in-memory demo download tasks (empty unless in demo mode)
                demoDownloads.getTasks().forEach((task, asin) => {
                    taskMap.set(asin, task);
                });

                setDownloadTasks(taskMap);
                setStageProgress(getStageProgress());

                // Derive download speed (bytes/sec) from the change in bytes between polls.
                const now = Date.now();
                const speeds: Record<string, number> = {};
                const prev = prevBytesRef.current;
                const nextPrev = new Map<string, {bytes: number; time: number}>();
                taskMap.forEach((task, asin) => {
                    if (task.status === 'downloading') {
                        const p = prev.get(asin);
                        if (p && now > p.time && task.bytes_downloaded >= p.bytes) {
                            speeds[asin] = ((task.bytes_downloaded - p.bytes) * 1000) / (now - p.time);
                        }
                        nextPrev.set(asin, {bytes: task.bytes_downloaded, time: now});
                    }
                });
                prevBytesRef.current = nextPrev;
                setDownloadSpeeds(speeds);
            } catch (error) {
                console.error('[LibraryScreen] Error polling progress:', error);
            }
        };

        pollProgress();
        progressInterval.current = setInterval(pollProgress, 2000);

        return () => {
            if (progressInterval.current) {
                clearInterval(progressInterval.current);
            }
        };
    }, []);

    const loadPreferences = async () => {
        try {
            const prefsJson = await SecureStore.getItemAsync(LIBRARY_PREFS_KEY);
            if (prefsJson) {
                const prefs: LibraryPreferences = JSON.parse(prefsJson);
                setSortField(prefs.sortField);
                setSortDirection(prefs.sortDirection);
                const groupField = prefs.downloadedGroupSortField || (prefs.sortField !== 'downloaded' ? prefs.sortField : 'title');
                const groupDirection = prefs.downloadedGroupSortDirection || (prefs.sortField !== 'downloaded' ? prefs.sortDirection : 'asc');
                setDownloadedGroupSortField(groupField);
                setDownloadedGroupSortDirection(groupDirection);
                if (prefs.sourceFilter) setSourceFilter(prefs.sourceFilter);
            }
        } catch (error) {
            console.error('[LibraryScreen] Error loading preferences:', error);
        }
    };

    const savePreferences = async (
        field: SortField,
        direction: SortDirection,
        groupField: BookSortField = downloadedGroupSortField,
        groupDirection: SortDirection = downloadedGroupSortDirection
    ) => {
        try {
            const prefs: LibraryPreferences = {
                sortField: field,
                sortDirection: direction,
                downloadedGroupSortField: groupField,
                downloadedGroupSortDirection: groupDirection,
            };
            await SecureStore.setItemAsync(LIBRARY_PREFS_KEY, JSON.stringify(prefs));
        } catch (error) {
            console.error('[LibraryScreen] Error saving preferences:', error);
        }
    };

    const loadFilterOptions = async () => {
        try {
            // Demo mode: filter options come from the in-memory dataset.
            const selectedId = await SecureStore.getItemAsync(SELECTED_ACCOUNT_KEY);
            if (isDemoAccountId(selectedId)) {
                setAllSeries(DEMO_SERIES);
                setAllCategories(DEMO_CATEGORIES);
                setAllAccounts([DEMO_ACCOUNT]);
                return;
            }

            const dbPath = getDatabasePath();

            try {
                initializeDatabase(dbPath);
                const series = getAllSeries(dbPath);
                const categories = getAllCategories(dbPath);
                const accounts = await getAllAccounts(dbPath);

                setAllSeries(series);
                setAllCategories(categories);
                setAllAccounts(accounts);
            } catch (error) {
                console.log('[LibraryScreen] Database not ready yet');
            }
        } catch (error) {
            console.error('[LibraryScreen] Error loading filter options:', error);
        }
    };

    const loadBooks = async (reset: boolean = false) => {
        try {
            const offset = reset ? 0 : audiobooks.length;
            const limit = PAGE_SIZE;

            // Demo mode: serve the in-memory dataset instead of querying SQLite.
            const selectedId = await SecureStore.getItemAsync(SELECTED_ACCOUNT_KEY);
            if (isDemoAccountId(selectedId)) {
                const result = filterSortPaginate(DEMO_BOOKS, {
                    offset,
                    limit,
                    searchQuery,
                    series: selectedSeries,
                    category: selectedCategories,
                    sortField,
                    sortDirection,
                });
                if (reset) {
                    setAudiobooks(result.books);
                } else {
                    setAudiobooks(prev => [...prev, ...result.books]);
                }
                setTotalCount(result.total_count);
                setHasMore(offset + result.books.length < result.total_count);
                return;
            }

            const dbPath = getDatabasePath();

            console.log('[LibraryScreen] Loading books from:', dbPath);

            try {
                initializeDatabase(dbPath);
            } catch (dbError) {
                console.log('[LibraryScreen] Database not initialized yet');
                setAudiobooks([]);
                setTotalCount(0);
                setHasMore(false);
                return;
            }

            console.log('[LibraryScreen] Fetching books:', {
                offset,
                limit,
                searchQuery,
                sortField,
                sortDirection,
                selectedSeries,
                selectedCategories,
                accountFilters,
                includePodcasts,
                downloadedGroupSortField,
                downloadedGroupSortDirection,
            });

            const response = getBooksWithFilters(
                dbPath,
                offset,
                limit,
                searchQuery || null,
                selectedSeries,
                selectedCategories,
                sortField,
                sortDirection,
                sourceFilter === 'all' ? null : sourceFilter,
                sortField === 'downloaded' ? downloadedGroupSortField : null,
                sortField === 'downloaded' ? downloadedGroupSortDirection : null,
                accountFilters,
                typeFilter === 'audiobooks' ? false : includePodcasts,
                null,
                typeFilter === 'podcasts'
            );

            console.log('[LibraryScreen] Loaded books:', response.books.length, 'of', response.total_count);

            if (reset) {
                setAudiobooks(response.books);
            } else {
                setAudiobooks(prev => [...prev, ...response.books]);
            }

            setTotalCount(response.total_count);
            setHasMore(offset + response.books.length < response.total_count);
        } catch (error) {
            console.error('[LibraryScreen] Error loading books:', error);
            if (reset) {
                setAudiobooks([]);
                setTotalCount(0);
            }
            setHasMore(false);
        } finally {
            setIsLoading(false);
            setIsRefreshing(false);
            setIsLoadingMore(false);
        }
    };

    const handleRefresh = () => {
        setIsRefreshing(true);
        setHasMore(true);
        loadBooks(true);
    };

    const handleLoadMore = () => {
        if (!isLoadingMore && !isLoading && hasMore) {
            console.log('[LibraryScreen] Loading more books...');
            setIsLoadingMore(true);
            loadBooks(false);
        }
    };

    const loadPodcastEpisodes = async (podcast: Book, reset: boolean = false) => {
        try {
            const dbPath = getDatabasePath();

            try {
                initializeDatabase(dbPath);
            } catch (dbError) {
                setPodcastEpisodes([]);
                setPodcastEpisodeCount(0);
                setHasMorePodcastEpisodes(false);
                return;
            }

            const offset = reset ? 0 : podcastEpisodes.length;
            let remoteEpisodeCount: number | null = null;

            try {
                const accountId = podcast.account?.split(',').find(Boolean) || singleAccountFilter;
                const account = accountId
                    ? await getAccount(dbPath, accountId)
                    : await getPrimaryAccount(dbPath);

                if (account) {
                    const syncStats = await syncPodcastEpisodes(
                        dbPath,
                        account,
                        podcast.audible_product_id,
                        offset,
                        PAGE_SIZE
                    );
                    remoteEpisodeCount = syncStats.total_library_count;
                }
            } catch (syncError) {
                console.warn('[LibraryScreen] Could not sync podcast episode page:', syncError);
            }

            const response = getBooksWithFilters(
                dbPath,
                offset,
                PAGE_SIZE,
                null,
                null,
                null,
                'release_date',
                'desc',
                'audible',
                null,
                null,
                accountFilters,
                true,
                podcast.audible_product_id
            );
            const totalEpisodes = Math.max(response.total_count, remoteEpisodeCount ?? 0);

            if (reset) {
                setPodcastEpisodes(response.books);
            } else {
                setPodcastEpisodes(prev => [...prev, ...response.books]);
            }

            setPodcastEpisodeCount(totalEpisodes);
            setHasMorePodcastEpisodes(offset + response.books.length < totalEpisodes);
        } catch (error) {
            console.error('[LibraryScreen] Error loading podcast episodes:', error);
            if (reset) {
                setPodcastEpisodes([]);
                setPodcastEpisodeCount(0);
            }
            setHasMorePodcastEpisodes(false);
        } finally {
            setIsLoadingPodcastEpisodes(false);
            setIsRefreshingPodcastEpisodes(false);
            setIsLoadingMorePodcastEpisodes(false);
        }
    };

    const handlePodcastPress = (podcast: Book) => {
        setSelectedPodcast(podcast);
        setPodcastEpisodes([]);
        setPodcastEpisodeCount(0);
        setHasMorePodcastEpisodes(true);
        setIsLoadingPodcastEpisodes(true);
        loadPodcastEpisodes(podcast, true);
    };

    const handleClosePodcast = () => {
        setSelectedPodcast(null);
        setPodcastEpisodes([]);
        setPodcastEpisodeCount(0);
        setHasMorePodcastEpisodes(false);
    };

    const handlePodcastEpisodesRefresh = () => {
        if (!selectedPodcast) return;

        setIsRefreshingPodcastEpisodes(true);
        loadPodcastEpisodes(selectedPodcast, true);
    };

    const handleLoadMorePodcastEpisodes = () => {
        if (!selectedPodcast || isLoadingPodcastEpisodes || isLoadingMorePodcastEpisodes || !hasMorePodcastEpisodes) {
            return;
        }

        setIsLoadingMorePodcastEpisodes(true);
        loadPodcastEpisodes(selectedPodcast, false);
    };

    const handleDownloadAllEpisodes = async () => {
        if (!selectedPodcast) return;

        try {
            const downloadDir = await SecureStore.getItemAsync(DOWNLOAD_PATH_KEY);
            if (!downloadDir) {
                Alert.alert(
                    'Download Directory Not Set',
                    'Please go to Settings and choose a download directory first.',
                    [{ text: 'OK' }]
                );
                return;
            }

            const episodesToDownload = podcastEpisodes.filter(episode =>
                episode.is_downloadable !== false
                && !episode.file_path
                && !downloadTasks.has(episode.audible_product_id)
            );

            if (episodesToDownload.length === 0) {
                Alert.alert(
                    'Nothing to Download',
                    'All loaded episodes are already downloaded or in the queue.'
                );
                return;
            }

            const hasPermission = await requestNotificationPermission();
            if (!hasPermission) {
                Alert.alert(
                    'Permission Required',
                    'Please grant notification permission to see download progress',
                    [{ text: 'OK' }]
                );
                return;
            }

            const dbPath = getDatabasePath();
            initializeDatabase(dbPath);

            const owningAccountId = singleAccountFilter || getBookAccountIds(selectedPodcast)[0] || null;
            let account = owningAccountId
                ? await getAccount(dbPath, owningAccountId)
                : await getPrimaryAccount(dbPath);

            if (!account) {
                account = await getPrimaryAccount(dbPath);
            }

            if (!account) {
                Alert.alert('Error', 'Please log in first');
                return;
            }

            if (account.identity?.access_token) {
                const expiresAt = new Date(account.identity.access_token.expires_at);
                const minutesUntilExpiry = (expiresAt.getTime() - Date.now()) / 1000 / 60;

                if (minutesUntilExpiry < 5) {
                    try {
                        const newTokens = await refreshToken(account);
                        account.identity.access_token.token = newTokens.access_token;
                        if (newTokens.refresh_token) {
                            account.identity.refresh_token = newTokens.refresh_token;
                        }
                        account.identity.access_token.expires_at = new Date(Date.now() + parseInt(newTokens.expires_in.toString()) * 1000).toISOString();
                        await saveAccount(dbPath, account);
                    } catch (refreshError) {
                        console.error('[LibraryScreen] Token refresh failed:', refreshError);
                        Alert.alert('Error', 'Please log in again - token refresh failed');
                        return;
                    }
                }
            }

            let queued = 0;
            for (const episode of episodesToDownload) {
                try {
                    const author = (episode.authors?.length || 0) > 0 ? episode.authors.join(', ') : undefined;
                    await enqueueDownloadNew(
                        episode.audible_product_id,
                        episode.title,
                        author,
                        account,
                        downloadDir,
                        'High'
                    );
                    queued++;
                } catch (episodeError: any) {
                    console.error('[LibraryScreen] Failed to enqueue episode:', episode.title, episodeError);
                }
            }

            Alert.alert(
                'Downloads Started',
                `${queued} episode${queued === 1 ? '' : 's'} added to the download queue.`
            );
        } catch (error: any) {
            console.error('[LibraryScreen] Download all error:', error);
            Alert.alert('Download Failed', error.message || 'Unknown error');
        }
    };

    const handleSortChange = (field: SortField, direction: SortDirection) => {
        let nextGroupField = downloadedGroupSortField;
        let nextGroupDirection = downloadedGroupSortDirection;
        if (field !== 'downloaded') {
            nextGroupField = field;
            nextGroupDirection = direction;
            setDownloadedGroupSortField(nextGroupField);
            setDownloadedGroupSortDirection(nextGroupDirection);
        }

        setSortField(field);
        setSortDirection(direction);
        savePreferences(field, direction, nextGroupField, nextGroupDirection);
        setShowSortModal(false);
    };

    const handleToggleSearchControls = () => {
        const nextValue = !showControls;
        setShowControls(nextValue);
        if (nextValue) {
            setShowExportControls(false);
        }
    };

    const handleToggleExportControls = () => {
        const nextValue = !showExportControls;
        setShowExportControls(nextValue);
        if (nextValue) {
            setShowControls(false);
        }
    };

    const handleToggleExportFormat = (format: LibraryExportFormat) => {
        setExportFormats(previousFormats => {
            if (previousFormats.includes(format)) {
                return previousFormats.filter(value => value !== format);
            }
            return [...previousFormats, format];
        });
    };

    const loadBooksForExport = (): Book[] => {
        const dbPath = getDatabasePath();
        initializeDatabase(dbPath);

        const pageSize = 500;
        const books: Book[] = [];
        let total = 0;

        do {
            const response = getBooksWithFilters(
                dbPath,
                books.length,
                pageSize,
                searchQuery || null,
                selectedSeries,
                selectedCategories,
                exportSortField === 'title' ? 'title' : 'length',
                exportSortDirection,
                sourceFilter === 'all' ? null : sourceFilter,
                null,
                null,
                accountFilters,
                typeFilter === 'audiobooks' ? false : includePodcasts,
                null,
                typeFilter === 'podcasts'
            );

            books.push(...response.books);
            total = response.total_count;
            if (response.books.length === 0) break;
        } while (books.length < total);

        return books;
    };

    const handleExportLibrary = async () => {
        if (exportFormats.length === 0) {
            Alert.alert('Export Library', 'Choose at least one format.');
            return;
        }

        try {
            setIsExporting(true);
            const books = loadBooksForExport();

            if (books.length === 0) {
                Alert.alert('Export Library', 'No audiobooks match the current search and filters.');
                return;
            }

            const directory = await Directory.pickDirectoryAsync(
                Platform.OS === 'android' ? undefined : Paths.document.uri
            );
            const files = await exportLibrary(books, directory, {
                formats: exportFormats,
                sortField: exportSortField,
                sortDirection: exportSortDirection,
                groupByAuthor: exportGroupByAuthor,
                groupBySeries: exportGroupBySeries,
            });

            Alert.alert(
                'Export Complete',
                `Saved ${files.length} file${files.length === 1 ? '' : 's'}:\n\n${files.map(file => file.name).join('\n')}`
            );
            setShowExportControls(false);
        } catch (error: any) {
            const message = error?.message || String(error);
            if (message.toLowerCase().includes('cancel')) {
                return;
            }
            console.error('[LibraryScreen] Export error:', error);
            Alert.alert('Export Failed', message);
        } finally {
            setIsExporting(false);
        }
    };

    const handleCopyExportText = async () => {
        try {
            setIsCopyingExportText(true);
            const books = loadBooksForExport();

            if (books.length === 0) {
                Alert.alert('Copy Export Text', 'No audiobooks match the current search and filters.');
                return;
            }

            const text = buildLibraryExportText(books, {
                formats: ['txt'],
                sortField: exportSortField,
                sortDirection: exportSortDirection,
                groupByAuthor: exportGroupByAuthor,
                groupBySeries: exportGroupBySeries,
            });

            copyTextToClipboard(text);
            Alert.alert('Copied', `Formatted TXT export for ${books.length} audiobook${books.length === 1 ? '' : 's'} copied to clipboard.`);
        } catch (error: any) {
            console.error('[LibraryScreen] Copy export text error:', error);
            Alert.alert('Copy Failed', error?.message || String(error));
        } finally {
            setIsCopyingExportText(false);
        }
    };

    const handleClearFilters = () => {
        setSearchQuery('');
        setSelectedSeries([]);
        setSelectedCategories([]);
        setSourceFilter('all');
        setTypeFilter('all');
        setAccountFilters([]);
        setShowFilterModal(false);
    };

    // Convert Audible's HTML descriptions (<p>/<br> structure) to plain text
    // while keeping the paragraph and line breaks.
    const stripHtml = (value?: string): string => {
        return (value || '')
            .replace(/<br\s*\/?>/gi, '\n')
            .replace(/<\/p>/gi, '\n\n')
            .replace(/<[^>]*>/g, '')
            .replace(/&amp;/g, '&')
            .replace(/&quot;/g, '"')
            .replace(/&#39;|&apos;/g, "'")
            .replace(/&nbsp;/g, ' ')
            .replace(/[ \t]+/g, ' ')
            .replace(/ ?\n ?/g, '\n')
            .replace(/\n{3,}/g, '\n\n')
            .trim();
    };

    const formatDuration = (seconds: number): string => {
        const hours = Math.floor(seconds / 3600);
        const minutes = Math.floor((seconds % 3600) / 60);
        return `${hours}h ${minutes}m`;
    };

    const getCoverUrl = (book: Book): string | null => {
        if (!book.cover_url) return null;
        return book.cover_url.replace(/_SL\d+_/, '_SL150_');
    };

    const formatEtaShort = (sec: number): string => {
        if (!sec || sec <= 0) return '';
        const h = Math.floor(sec / 3600);
        const m = Math.floor((sec % 3600) / 60);
        const s = Math.floor(sec % 60);
        if (h > 0) return `~${h}h ${m}m`;
        if (m > 0) return `~${m}m ${s}s`;
        return `~${s}s`;
    };

    const formatSpeed = (bytesPerSec: number): string => {
        if (!bytesPerSec || bytesPerSec <= 0) return '';
        const mb = bytesPerSec / (1024 * 1024);
        if (mb >= 1) return `${mb.toFixed(1)} MB/s`;
        return `${(bytesPerSec / 1024).toFixed(0)} KB/s`;
    };

    const getStatus = (book: Book): { text: string; color: string } => {
        // Stage percentage + ETA come from the live store; the stage itself is the DB
        // task status (both foreground and auto-download persist it to the DB row).
        const sp = stageProgress[book.audible_product_id];

        // Sequential-mode queued books have no DB task yet; surface them so they read
        // as queued instead of available and can't be accidentally re-selected.
        if (sp && sp.stage === 'queued') {
            return {text: '⏳ Queued', color: colors.textSecondary};
        }

        const task = downloadTasks.get(book.audible_product_id);

        if (task) {
            const percentage = task.total_bytes > 0
                ? ((task.bytes_downloaded / task.total_bytes) * 100).toFixed(1)
                : '0.0';

            const etaText = sp ? formatEtaShort(sp.eta_seconds) : '';
            const etaSuffix = etaText ? ` · ${etaText}` : '';
            const stagePct = sp ? Math.round(sp.percentage) : 0;
            const speedText = formatSpeed(downloadSpeeds[book.audible_product_id]);
            const speedSuffix = speedText ? ` · ${speedText}` : '';

            switch (task.status) {
                case 'queued':
                    return {text: '⏳ Queued', color: colors.textSecondary};
                case 'downloading':
                    return {text: `⬇ ${percentage}%${speedSuffix}${etaSuffix}`, color: colors.info};
                case 'paused':
                    return {text: `⏸ Paused ${percentage}%`, color: colors.warning};
                case 'decrypting':
                    return {text: `🔓 Decrypting ${stagePct}%${etaSuffix}`, color: colors.info};
                case 'validating':
                    return {text: `🔍 Validating ${stagePct}%${etaSuffix}`, color: colors.info};
                case 'copying':
                    return {text: `📁 Saving ${stagePct}%${etaSuffix}`, color: colors.info};
                case 'completed':
                    return {text: '✓ Downloaded', color: colors.success};
                case 'failed':
                    return {text: '✗ Failed', color: colors.error};
                default:
                    return {text: 'Available', color: colors.textSecondary};
            }
        }

        if (book.file_path) {
            return {text: '✓ Downloaded', color: colors.success};
        }

        if (book.is_downloadable === false) {
            return {text: 'Episodes Only', color: colors.textSecondary};
        }

        return {text: 'Available', color: colors.textSecondary};
    };

    const isPodcastParent = (book: Book): boolean => {
        const deliveryType = book.content_delivery_type?.toLowerCase() || '';
        return book.content_type === 4
            || (book.is_downloadable === false && (
                deliveryType.includes('podcast')
                || deliveryType === 'periodical'
            ));
    };

    const getBookAccountIds = (book: Book): string[] => {
        return (book.account || '')
            .split(',')
            .map(value => value.trim())
            .filter(Boolean);
    };

    const getBookAccountLabels = (book: Book): string[] => {
        return getBookAccountIds(book).map((accountId) => {
            const savedAccount = allAccounts.find((a) => a.account_id === accountId);
            return savedAccount?.account_name || accountId;
        });
    };

    const requestNotificationPermission = async (): Promise<boolean> => {
        if (Platform.OS === 'android') {
            if (Platform.Version >= 33) {
                try {
                    const granted = await PermissionsAndroid.request(
                        PermissionsAndroid.PERMISSIONS.POST_NOTIFICATIONS,
                        {
                            title: 'Notification Permission',
                            message: 'LibriSync needs notification permission to show download progress',
                            buttonPositive: 'OK',
                        }
                    );
                    return granted === PermissionsAndroid.RESULTS.GRANTED;
                } catch (err) {
                    console.warn('[LibraryScreen] Notification permission error:', err);
                    return false;
                }
            }
            return true;
        }
        return true;
    };

    // Returns true when a download was actually started/enqueued, false on any refusal or
    // failure — the batch mode uses this for an honest "Started X of N" summary.
    const handleDownload = async (book: Book, silent = false): Promise<boolean> => {
        // Guard against re-selecting a book that is already downloading or queued.
        const existingStage = stageProgress[book.audible_product_id];
        const existingTask = downloadTasks.get(book.audible_product_id);
        // Block only when it is genuinely running: a live stage in the store, or an active
        // download-task status. Cancelled / failed / interrupted (stale decrypt/copy) tasks
        // stay retriable.
        const activeStatuses = ['downloading', 'paused', 'queued'];
        if (existingStage || (existingTask && activeStatuses.includes(existingTask.status))) {
            if (!silent) Alert.alert('Already in Progress', `"${book.title}" is already downloading or queued.`);
            return false;
        }
        try {
            // Demo mode: real LibriVox MP3 download to the app sandbox (no SAF dir,
            // no DRM, no native pipeline). Progress shows through the same UI.
            if (isDemoBook(book)) {
                demoDownloads.enqueue(book);
                if (!silent) Alert.alert(
                    'Download Started',
                    `"${book.title}" is downloading. Watch the progress here in the library.`
                );
                return true;
            }

            const downloadDir = await SecureStore.getItemAsync(DOWNLOAD_PATH_KEY);
            if (!downloadDir) {
                if (!silent) Alert.alert(
                    'Download Directory Not Set',
                    'Please go to Settings and choose a download directory first.',
                    [{ text: 'OK' }]
                );
                return false;
            }

            // LibriVox books: background download, no auth needed
            if (book.source === 'librivox') {
                console.log('[LibraryScreen] LibriVox download:', book.title);
                const librivoxId = book.audible_product_id.replace('librivox_', '');
                const authorText = Array.isArray(book.authors) ? book.authors.join(', ') : (book.authors || 'Unknown Author');

                try {
                    const librivoxBook = await getBook(librivoxId);
                    if (librivoxBook?.url_zip_file) {
                        await downloadLibrivoxFile(
                            librivoxId,
                            book.title,
                            authorText,
                            librivoxBook.url_zip_file,
                            downloadDir
                        );
                        if (!silent) Alert.alert(
                            'Download Started',
                            `"${book.title}" is downloading. Check the notification for progress.`
                        );
                        return true;
                    }
                    console.warn('[LibraryScreen] No download URL for LibriVox book:', book.audible_product_id);
                    if (!silent) Alert.alert('Error', 'Could not find download URL for this book.');
                } catch (dlError: any) {
                    console.error('[LibraryScreen] LibriVox download error:', dlError);
                    if (!silent) Alert.alert('Download Failed', dlError.message || 'Unknown error');
                }
                return false;
            }

            // Audible books: existing DRM download flow
            if (book.is_downloadable === false) {
                if (!silent) Alert.alert(
                    'Not Downloadable',
                    'This item is a podcast or periodical parent. Enable podcasts to sync episodes, then download an episode instead.'
                );
                return false;
            }

            const hasPermission = await requestNotificationPermission();
            if (!hasPermission) {
                if (!silent) Alert.alert(
                    'Permission Required',
                    'Please grant notification permission to see download progress',
                    [{ text: 'OK' }]
                );
                return false;
            }

            const dbPath = getDatabasePath();
            initializeDatabase(dbPath);

            const owningAccountId = singleAccountFilter || getBookAccountIds(book)[0] || null;
            let account = owningAccountId
                ? await getAccount(dbPath, owningAccountId)
                : await getPrimaryAccount(dbPath);

            if (!account) {
                account = await getPrimaryAccount(dbPath);
            }

            if (!account) {
                if (!silent) Alert.alert('Error', 'Please log in first');
                return false;
            }

            if (account.identity?.access_token) {
                const expiresAt = new Date(account.identity.access_token.expires_at);
                const now = new Date();
                const minutesUntilExpiry = (expiresAt.getTime() - now.getTime()) / 1000 / 60;

                if (minutesUntilExpiry < 5) {
                    console.log('[LibraryScreen] Token expiring soon, refreshing...');
                    try {
                        const newTokens = await refreshToken(account);
                        account.identity.access_token.token = newTokens.access_token;
                        if (newTokens.refresh_token) {
                            account.identity.refresh_token = newTokens.refresh_token;
                        }
                        const newExpiresAt = new Date(Date.now() + parseInt(newTokens.expires_in.toString()) * 1000).toISOString();
                        account.identity.access_token.expires_at = newExpiresAt;

                        await saveAccount(dbPath, account);
                        console.log('[LibraryScreen] Token refreshed successfully');
                    } catch (refreshError) {
                        console.error('[LibraryScreen] Token refresh failed:', refreshError);
                        if (!silent) Alert.alert('Error', 'Please log in again - token refresh failed');
                        return false;
                    }
                }
            }

            console.log('[LibraryScreen] Enqueueing download:', book.title, book.audible_product_id);

            // Retry of a cancelled/failed/interrupted book: clear the stale task first so a
            // fresh download starts cleanly (keep any partial file off the record).
            if (existingTask) {
                try {
                    await clearBookDownloadState(dbPath, book.audible_product_id, false);
                } catch (clearError) {
                    console.warn('[LibraryScreen] Failed to clear stale download state:', clearError);
                }
            }

            const author = (book.authors?.length || 0) > 0 ? book.authors.join(', ') : undefined;

            await enqueueDownloadNew(
                book.audible_product_id,
                book.title,
                author,
                account,
                downloadDir,
                'High'
            );

            console.log('[LibraryScreen] Download enqueued successfully');

            if (!silent) Alert.alert(
                'Download Started',
                `"${book.title}" is downloading. Monitor progress here or in the notification — you can leave the app.`
            );
            return true;

        } catch (error: any) {
            console.error('[LibraryScreen] Download error:', error);
            if (!silent) Alert.alert('Download Failed', error.message || 'Unknown error');
            return false;
        }
    };

    const handlePauseDownload = (book: Book) => {
        try {
            if (isDemoBook(book)) {
                demoDownloads.pause(book.audible_product_id);
                return;
            }

            const dbPath = getDatabasePath();

            const task = downloadTasks.get(book.audible_product_id);
            if (task) {
                pauseDownload(dbPath, task.task_id);
                console.log('[LibraryScreen] Paused download:', book.title);
            }
        } catch (error) {
            console.error('[LibraryScreen] Pause error:', error);
        }
    };

    const handleResumeDownload = (book: Book) => {
        try {
            if (isDemoBook(book)) {
                demoDownloads.resume(book.audible_product_id);
                return;
            }

            const dbPath = getDatabasePath();

            const task = downloadTasks.get(book.audible_product_id);
            if (task) {
                resumeDownload(dbPath, task.task_id);
                console.log('[LibraryScreen] Resumed download:', book.title);
            }
        } catch (error) {
            console.error('[LibraryScreen] Resume error:', error);
        }
    };

    const handleCancelDownload = (book: Book) => {
        try {
            if (isDemoBook(book)) {
                Alert.alert(
                    'Cancel Download',
                    `Are you sure you want to cancel downloading "${book.title}"?`,
                    [
                        { text: 'No', style: 'cancel' },
                        {
                            text: 'Yes',
                            style: 'destructive',
                            onPress: () => demoDownloads.cancel(book.audible_product_id),
                        }
                    ]
                );
                return;
            }

            const dbPath = getDatabasePath();
            const task = downloadTasks.get(book.audible_product_id);
            const isQueued = stageProgress[book.audible_product_id]?.stage === 'queued';

            Alert.alert(
                isQueued ? 'Remove from Queue' : 'Cancel Download',
                isQueued
                    ? `Remove "${book.title}" from the download queue?`
                    : `Are you sure you want to cancel downloading "${book.title}"?`,
                [
                    { text: 'No', style: 'cancel' },
                    {
                        text: 'Yes',
                        style: 'destructive',
                        onPress: () => {
                            // Cancel the native task if it is already running, then run the
                            // service-side cleanup (notification, active + pending queue,
                            // advance) which also removes a still-queued book.
                            if (task) cancelDownload(dbPath, task.task_id);
                            stopDownloadMonitoring(book.audible_product_id);
                            console.log('[LibraryScreen] Cancelled/removed download:', book.title);
                        }
                    }
                ]
            );
        } catch (error) {
            console.error('[LibraryScreen] Cancel error:', error);
        }
    };

    const handleRetryConversion = async (book: Book) => {
        try {
            const dbPath = getDatabasePath();

            await retryConversion(dbPath, book.audible_product_id);
            console.log('[LibraryScreen] Conversion retry started:', book.title);
        } catch (error: any) {
            console.error('[LibraryScreen] Retry conversion error:', error);
            Alert.alert('Retry Failed', error.message || 'Failed to retry conversion');
        }
    };

    const refreshAfterDownloadStateChange = (book: Book) => {
        loadBooks(true);

        if (
            selectedPodcast &&
            book.origin_asin === selectedPodcast.audible_product_id
        ) {
            setPodcastEpisodes(prev =>
                prev.map(episode =>
                    episode.audible_product_id === book.audible_product_id
                        ? {...episode, file_path: undefined}
                        : episode
                )
            );
            loadPodcastEpisodes(selectedPodcast, true);
        }
    };

    const handleMarkAsNotDownloaded = async (book: Book) => {
        try {
            const dbPath = getDatabasePath();

            // Check if file exists
            const filePath = await getBookFilePath(dbPath, book.audible_product_id);

            if (filePath) {
                // File exists - show options to delete or just clear database
                Alert.alert(
                    'Mark as Not Downloaded',
                    `A downloaded file exists for "${book.title}".\n\nWhat would you like to do?`,
                    [
                        { text: 'Cancel', style: 'cancel' },
                        {
                            text: 'Clear Status Only',
                            onPress: async () => {
                                try {
                                    await clearBookDownloadState(dbPath, book.audible_product_id, false);
                                    console.log('[LibraryScreen] Cleared download status:', book.title);
                                    Alert.alert('Success', `Download status cleared for "${book.title}".\n\nThe file still exists on disk.`);
                                    refreshAfterDownloadStateChange(book);
                                } catch (error: any) {
                                    console.error('[LibraryScreen] Clear status error:', error);
                                    Alert.alert('Error', error.message || 'Failed to clear download status');
                                }
                            }
                        },
                        {
                            text: 'Delete File',
                            style: 'destructive',
                            onPress: async () => {
                                try {
                                    const result = await clearBookDownloadState(dbPath, book.audible_product_id, true);
                                    if (result.file_deleted) {
                                        console.log('[LibraryScreen] Deleted file and cleared status:', {
                                            title: book.title,
                                            coverDeleted: result.cover_deleted,
                                            bookFolderDeleted: result.book_folder_deleted,
                                            authorFolderDeleted: result.author_folder_deleted,
                                            cleanupError: result.cleanup_error,
                                        });
                                        if (result.cleanup_error) {
                                            Alert.alert('Partial Cleanup', `File deleted and download status cleared for "${book.title}".\n\nCleanup error: ${result.cleanup_error}`);
                                        } else {
                                            Alert.alert('Success', `File deleted and download status cleared for "${book.title}".`);
                                        }
                                    } else {
                                        console.warn('[LibraryScreen] Cleared status but file delete failed:', {
                                            title: book.title,
                                            error: result.delete_error,
                                        });
                                        const deleteError = result.delete_error ? `\n\nDelete error: ${result.delete_error}` : '';
                                        Alert.alert('Partial Success', `Download status cleared, but file could not be deleted.${deleteError}\n\nYou may need to delete it manually.`);
                                    }
                                    refreshAfterDownloadStateChange(book);
                                } catch (error: any) {
                                    console.error('[LibraryScreen] Delete file error:', error);
                                    Alert.alert('Error', error.message || 'Failed to delete file');
                                }
                            }
                        }
                    ]
                );
            } else {
                // No file exists - just clear database
                Alert.alert(
                    'Mark as Not Downloaded',
                    `Mark "${book.title}" as not downloaded?\n\nThis will clear its download status.`,
                    [
                        { text: 'Cancel', style: 'cancel' },
                        {
                            text: 'Clear Status',
                            style: 'destructive',
                            onPress: async () => {
                                try {
                                    await clearBookDownloadState(dbPath, book.audible_product_id, false);
                                    console.log('[LibraryScreen] Cleared download status:', book.title);
                                    Alert.alert('Success', `Download status cleared for "${book.title}".`);
                                    refreshAfterDownloadStateChange(book);
                                } catch (error: any) {
                                    console.error('[LibraryScreen] Clear status error:', error);
                                    Alert.alert('Error', error.message || 'Failed to clear download status');
                                }
                            }
                        }
                    ]
                );
            }
        } catch (error) {
            console.error('[LibraryScreen] Mark as not downloaded error:', error);
        }
    };

    const handleSelectFileAsDownloaded = async (book: Book) => {
        try {
            // Open file picker for audio files
            const result = await DocumentPicker.getDocumentAsync({
                type: ['audio/*', 'application/octet-stream'],
                copyToCacheDirectory: false,
            });

            if (result.canceled || !result.assets || result.assets.length === 0) {
                console.log('[LibraryScreen] File picker cancelled');
                return;
            }

            const file = result.assets[0];
            const dbPath = getDatabasePath();

            console.log('[LibraryScreen] Selected file:', file.uri);

            await setBookFilePath(dbPath, book.audible_product_id, book.title, file.uri);

            Alert.alert(
                'Success',
                `"${book.title}" has been marked as downloaded.\n\nFile: ${file.name}`
            );

            loadBooks(true);
        } catch (error: any) {
            console.error('[LibraryScreen] Set file path error:', error);
            Alert.alert('Error', error.message || 'Failed to set file path');
        }
    };

    const handleCreateCoverArt = async (book: Book) => {
        try {
            if (!book.cover_url) {
                Alert.alert('Error', 'This book has no cover image available');
                return;
            }

            // Get the book's file path
            const dbPath = getDatabasePath();

            const filePath = await getBookFilePath(dbPath, book.audible_product_id);

            if (!filePath) {
                Alert.alert(
                    'Error',
                    'This book is not downloaded yet. Please download or select a file first.'
                );
                return;
            }

            Alert.alert(
                'Creating Cover Art',
                'Downloading and creating EmbeddedCover.jpg...',
                []
            );

            const result = await createCoverArtFile(
                book.audible_product_id,
                book.cover_url,
                filePath
            );

            Alert.alert(
                'Success',
                `Cover art created successfully!\n\nEmbeddedCover.jpg (500x500) has been saved in the same directory as your audiobook.`
            );
        } catch (error: any) {
            console.error('[LibraryScreen] Create cover art error:', error);
            Alert.alert('Error', error.message || 'Failed to create cover art');
        }
    };

    const handleBookLongPress = (book: Book) => {
        setSelectedBook(book);
        setShowContextMenu(true);
    };

    const exitBatchMode = () => {
        setBatchMode(false);
        setSelectedForBatch(new Set());
    };

    const toggleBatchSelect = (asin: string) => {
        setSelectedForBatch(prev => {
            const next = new Set(prev);
            if (next.has(asin)) next.delete(asin);
            else next.add(asin);
            return next;
        });
    };

    const handleBatchDownload = async () => {
        const books = audiobooks.filter(b => selectedForBatch.has(b.audible_product_id));
        if (books.length === 0) return;
        // Precheck the download directory once so a batch of many books doesn't fail per-book.
        if (books.some(b => !isDemoBook(b))) {
            const dir = await SecureStore.getItemAsync(DOWNLOAD_PATH_KEY);
            if (!dir) {
                Alert.alert('Download Directory Not Set', 'Please choose a download directory in Settings first.');
                return;
            }
        }
        let started = 0;
        for (const b of books) {
            // silent: the batch shows one summary instead of per-book alerts
            if (await handleDownload(b, true)) started++;
        }
        const n = books.length;
        exitBatchMode();
        Alert.alert(
            'Batch Download',
            started === n
                ? `Started downloading ${n} book${n === 1 ? '' : 's'}.`
                : `Started ${started} of ${n} books. The rest were skipped or failed — check the library list.`
        );
    };

    const renderItem = ({item}: { item: Book }) => {
        const status = getStatus(item);
        const parentPodcast = isPodcastParent(item);
        const authorText = parentPodcast
            ? 'Podcast'
            : (item.authors?.length || 0) > 0 ? item.authors.join(', ') : 'Unknown Author';
        const coverUrl = getCoverUrl(item);
        const task = downloadTasks.get(item.audible_product_id);
        // Sequential-mode queued books have no DB task; they are queued only in the store.
        const isSpQueued = stageProgress[item.audible_product_id]?.stage === 'queued';
        // Only a saved final file means "downloaded". The download-task "completed"
        // status is also set once the download alone finishes (before decrypt/copy), so a
        // cancelled or interrupted book could otherwise look downloaded and block retry.
        const isDownloaded = !!item.file_path;
        const isProcessing = task?.status === 'decrypting' || task?.status === 'validating' || task?.status === 'copying';
        const canRetryConversion = task?.status === 'failed' && !!task.aaxc_key;
        const isDownloading = task?.status === 'downloading';
        const isPaused = task?.status === 'paused';
        const isQueued = task?.status === 'queued' || isSpQueued;
        const isActive = isDownloading || isPaused || isQueued || isProcessing;
        // Retriable whenever it is not actually downloaded and nothing is actively running:
        // covers cancelled, failed, interrupted, and a stale "completed" with no saved file.
        const canDownload = item.is_downloadable !== false && !isDownloaded && !isActive && !canRetryConversion;

        return (
            <TouchableOpacity
                style={[styles.item, batchMode && selectedForBatch.has(item.audible_product_id) && styles.itemSelected]}
                onPress={() => {
                    if (batchMode) {
                        if (canDownload) toggleBatchSelect(item.audible_product_id);
                        return;
                    }
                    setSelectedBook(item);
                    setDetailDescriptionExpanded(false);
                    setDetailDescriptionLines(0);
                    setShowDetailModal(true);
                }}
                onLongPress={() => handleBookLongPress(item)}
                accessibilityLabel={parentPodcast ? `Open ${item.title} episodes` : item.title}
            >
                <View style={styles.itemRow}>
                    {batchMode && (
                        <View style={[
                            styles.batchCheck,
                            selectedForBatch.has(item.audible_product_id) && styles.batchCheckOn,
                            !canDownload && styles.batchCheckDisabled,
                        ]}>
                            {selectedForBatch.has(item.audible_product_id) && (
                                <Text style={styles.batchCheckMark}>✓</Text>
                            )}
                        </View>
                    )}
                    {coverUrl ? (
                        <Image
                            source={{uri: coverUrl}}
                            style={styles.cover}
                            resizeMode="cover"
                        />
                    ) : (
                        <View style={styles.coverPlaceholder}>
                            <Text style={styles.coverPlaceholderText}>📚</Text>
                        </View>
                    )}
                    <View style={styles.itemContent}>
                        <Text style={styles.title} numberOfLines={2}>
                            {item.title}
                        </Text>
                        <Text style={styles.author} numberOfLines={1}>
                            {authorText}
                        </Text>
                        {item.series_name && (
                            <Text style={styles.series} numberOfLines={1}>
                                {item.series_name} {item.series_sequence ? `#${item.series_sequence}` : ''}
                            </Text>
                        )}
                        {item.episode_number !== undefined && item.episode_number !== null && (
                            <Text style={styles.series} numberOfLines={1}>
                                Episode {item.episode_number}
                            </Text>
                        )}
                        <View style={styles.metadata}>
                            {!parentPodcast && (
                                <Text style={styles.duration}>{formatDuration(item.duration_seconds)}</Text>
                            )}
                            {item.source === 'librivox' && (
                                <View style={styles.sourceBadge}>
                                    <Text style={styles.sourceBadgeText}>LibriVox</Text>
                                </View>
                            )}
                            {allAccounts.length > 1 && getBookAccountLabels(item).map((label) => (
                                <View key={label} style={styles.sourceBadge}>
                                    <Text style={styles.sourceBadgeText}>{label}</Text>
                                </View>
                            ))}
                            <Text style={[styles.status, {color: status.color}]}>
                                {status.text}
                            </Text>
                        </View>
                    </View>

                    {!batchMode && !isDownloaded && !isProcessing && canDownload && (
                        <TouchableOpacity
                            style={styles.downloadButton}
                            onPress={() => handleDownload(item)}
                        >
                            <Text style={styles.downloadButtonText}>⬇</Text>
                        </TouchableOpacity>
                    )}

                    {!isDownloaded && canRetryConversion && (
                        <TouchableOpacity
                            style={styles.resumeButton}
                            onPress={() => handleRetryConversion(item)}
                        >
                            <Text style={styles.resumeButtonText}>↻</Text>
                        </TouchableOpacity>
                    )}

                    {isProcessing && (
                        <View style={styles.downloadButton}>
                            <ActivityIndicator size="small" color={colors.info} />
                        </View>
                    )}

                    {isDownloading && (
                        <TouchableOpacity
                            style={styles.pauseButton}
                            onPress={() => handlePauseDownload(item)}
                        >
                            <Text style={styles.pauseButtonText}>⏸</Text>
                        </TouchableOpacity>
                    )}

                    {isPaused && (
                        <TouchableOpacity
                            style={styles.resumeButton}
                            onPress={() => handleResumeDownload(item)}
                        >
                            <Text style={styles.resumeButtonText}>▶</Text>
                        </TouchableOpacity>
                    )}

                    {(isDownloading || isPaused || isQueued || isProcessing) && (
                        <TouchableOpacity
                            style={styles.cancelButton}
                            onPress={() => handleCancelDownload(item)}
                        >
                            <Text style={styles.cancelButtonText}>✕</Text>
                        </TouchableOpacity>
                    )}

                    {isQueued && (
                        <View style={styles.downloadButton}>
                            <ActivityIndicator size="small" color={colors.textSecondary} />
                        </View>
                    )}

                    {parentPodcast && (
                        <View style={styles.episodeButton}>
                            <Ionicons name="list" size={22} color={colors.accent} />
                        </View>
                    )}
                </View>
            </TouchableOpacity>
        );
    };

    const renderPodcastEpisodeItem = ({item}: { item: Book }) => renderItem({item});

    const getSortLabel = () => {
        const fieldLabels = {
            title: 'Title',
            release_date: 'Release Date',
            date_added: 'Date Added',
            series: 'Series',
            length: 'Length',
            downloaded: 'Downloaded',
        };
        const arrow = sortDirection === 'asc' ? '↑' : '↓';
        if (sortField === 'downloaded') {
            const groupArrow = downloadedGroupSortDirection === 'asc' ? '↑' : '↓';
            return `Downloaded ${arrow} (${fieldLabels[downloadedGroupSortField]} ${groupArrow})`;
        }
        return `${fieldLabels[sortField]} ${arrow}`;
    };

    const COLLAPSED_FILTER_OPTIONS = 10;

    // Multi-select filter section: tap toggles a value, "All ..." clears the
    // section, long lists collapse behind a "Show all" row.
    const renderMultiSelectSection = (
        title: string,
        allLabel: string,
        options: string[],
        selected: string[],
        setSelected: (values: string[]) => void,
        sectionKey: string,
        getLabel: (value: string) => string = (value) => value
    ) => {
        const expanded = !!expandedFilterSections[sectionKey];
        // Keep selected options visible even when collapsed
        const visible = expanded || options.length <= COLLAPSED_FILTER_OPTIONS
            ? options
            : options.filter((option, index) => index < COLLAPSED_FILTER_OPTIONS || selected.includes(option));
        const toggle = (value: string) =>
            setSelected(
                selected.includes(value)
                    ? selected.filter((entry) => entry !== value)
                    : [...selected, value]
            );

        return (
            <>
                <Text style={styles.filterSectionTitle}>{title}</Text>
                <TouchableOpacity
                    style={[
                        styles.filterOption,
                        selected.length === 0 && styles.filterOptionSelected
                    ]}
                    onPress={() => setSelected([])}
                >
                    <Text style={styles.filterOptionText}>{allLabel}</Text>
                    {selected.length === 0 && <Text style={styles.modalCheck}>✓</Text>}
                </TouchableOpacity>
                {visible.map((option) => (
                    <TouchableOpacity
                        key={option}
                        style={[
                            styles.filterOption,
                            selected.includes(option) && styles.filterOptionSelected
                        ]}
                        onPress={() => toggle(option)}
                    >
                        <Text style={styles.filterOptionText}>{getLabel(option)}</Text>
                        {selected.includes(option) && <Text style={styles.modalCheck}>✓</Text>}
                    </TouchableOpacity>
                ))}
                {options.length > COLLAPSED_FILTER_OPTIONS && (
                    <TouchableOpacity
                        style={styles.filterOption}
                        onPress={() =>
                            setExpandedFilterSections((prev) => ({...prev, [sectionKey]: !expanded}))
                        }
                    >
                        <Text style={[styles.filterOptionText, {color: colors.accent}]}>
                            {expanded ? 'Show less' : `Show all (${options.length})`}
                        </Text>
                    </TouchableOpacity>
                )}
            </>
        );
    };

    const getActiveFiltersCount = () => {
        let count = 0;
        count += selectedSeries.length;
        count += selectedCategories.length;
        if (sourceFilter !== 'all') count++;
        if (typeFilter !== 'all') count++;
        count += accountFilters.length;
        return count;
    };

    const selectedBookIsPodcastParent = selectedBook ? isPodcastParent(selectedBook) : false;

    return (
        <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
            <View style={styles.header}>
                <View style={styles.headerTitleRow}>
                    <Text style={styles.headerTitle}>Library</Text>
                    <View style={styles.headerActions}>
                        <TouchableOpacity
                            style={[
                                styles.toggleControlsButton,
                                showExportControls && styles.toggleControlsButtonActive
                            ]}
                            onPress={handleToggleExportControls}
                            accessibilityLabel="Export library"
                        >
                            <Ionicons
                                name={showExportControls ? 'close' : 'download-outline'}
                                size={24}
                                color={colors.textPrimary}
                            />
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={[
                                styles.toggleControlsButton,
                                showControls && styles.toggleControlsButtonActive
                            ]}
                            onPress={handleToggleSearchControls}
                            accessibilityLabel="Search library"
                        >
                            <Ionicons
                                name={showControls ? 'close' : 'search'}
                                size={24}
                                color={colors.textPrimary}
                            />
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={[
                                styles.toggleControlsButton,
                                batchMode && styles.toggleControlsButtonActive
                            ]}
                            onPress={() => (batchMode ? exitBatchMode() : setBatchMode(true))}
                            accessibilityLabel="Batch download"
                        >
                            <Ionicons
                                name={batchMode ? 'close' : 'checkbox-outline'}
                                size={24}
                                color={colors.textPrimary}
                            />
                        </TouchableOpacity>
                    </View>
                </View>

                {batchMode && (
                    <View style={styles.batchBar}>
                        <Text style={styles.batchBarText}>{selectedForBatch.size} selected</Text>
                        <View style={styles.batchBarButtons}>
                            <TouchableOpacity
                                style={styles.batchBarButtonSecondary}
                                onPress={() => setSelectedForBatch(new Set())}
                                disabled={selectedForBatch.size === 0}
                            >
                                <Text style={styles.batchBarButtonSecondaryText}>Clear</Text>
                            </TouchableOpacity>
                            <TouchableOpacity
                                style={[styles.batchBarButton, selectedForBatch.size === 0 && styles.batchBarButtonDisabled]}
                                onPress={handleBatchDownload}
                                disabled={selectedForBatch.size === 0}
                            >
                                <Text style={styles.batchBarButtonText}>Download ({selectedForBatch.size})</Text>
                            </TouchableOpacity>
                        </View>
                    </View>
                )}

                {showControls && (
                    <>
                        {/* Search Bar */}
                        <View style={styles.searchContainer}>
                            <Ionicons
                                name="search"
                                size={20}
                                color={colors.textSecondary}
                                style={styles.searchIcon}
                            />
                            <TextInput
                                style={styles.searchInput}
                                placeholder="Search titles, authors, narrators, series..."
                                placeholderTextColor={colors.textSecondary}
                                value={searchQuery}
                                onChangeText={setSearchQuery}
                                returnKeyType="search"
                            />
                            {searchQuery.length > 0 && (
                                <TouchableOpacity onPress={() => setSearchQuery('')}>
                                    <Ionicons
                                        name="close-circle"
                                        size={20}
                                        color={colors.textSecondary}
                                    />
                                </TouchableOpacity>
                            )}
                        </View>

                        {/* Controls Row */}
                        <View style={styles.controlsRow}>
                            <TouchableOpacity
                                style={styles.controlButton}
                                onPress={() => setShowSortModal(true)}
                            >
                                <Text style={styles.controlButtonText}>
                                    {getSortLabel()}
                                </Text>
                            </TouchableOpacity>

                            <TouchableOpacity
                                style={[
                                    styles.controlButton,
                                    getActiveFiltersCount() > 0 && styles.controlButtonActive
                                ]}
                                onPress={() => setShowFilterModal(true)}
                            >
                                <Text style={styles.controlButtonText}>
                                    Filter {getActiveFiltersCount() > 0 ? `(${getActiveFiltersCount()})` : ''}
                                </Text>
                            </TouchableOpacity>
                        </View>
                    </>
                )}

                {showExportControls && (
                    <View style={styles.exportContainer}>
                        <Text style={styles.exportSectionLabel}>Formats</Text>
                        <View style={styles.exportFormatGrid}>
                            {EXPORT_FORMAT_OPTIONS.map(option => {
                                const selected = exportFormats.includes(option.format);
                                return (
                                    <TouchableOpacity
                                        key={option.format}
                                        style={[
                                            styles.exportFormatButton,
                                            selected && styles.exportOptionSelected
                                        ]}
                                        onPress={() => handleToggleExportFormat(option.format)}
                                    >
                                        <Ionicons
                                            name={option.icon}
                                            size={18}
                                            color={selected ? colors.accent : colors.textSecondary}
                                        />
                                        <Text style={styles.exportFormatText}>{option.label}</Text>
                                    </TouchableOpacity>
                                );
                            })}
                        </View>

                        <Text style={styles.exportSectionLabel}>Sort</Text>
                        <View style={styles.exportSegmentedRow}>
                            <TouchableOpacity
                                style={[
                                    styles.exportSegmentButton,
                                    exportSortField === 'title' && styles.exportOptionSelected
                                ]}
                                onPress={() => setExportSortField('title')}
                            >
                                <Text style={styles.exportSegmentText}>Name</Text>
                            </TouchableOpacity>
                            <TouchableOpacity
                                style={[
                                    styles.exportSegmentButton,
                                    exportSortField === 'length' && styles.exportOptionSelected
                                ]}
                                onPress={() => setExportSortField('length')}
                            >
                                <Text style={styles.exportSegmentText}>Length</Text>
                            </TouchableOpacity>
                            <TouchableOpacity
                                style={styles.exportDirectionButton}
                                onPress={() => setExportSortDirection(exportSortDirection === 'asc' ? 'desc' : 'asc')}
                            >
                                <Ionicons
                                    name={exportSortDirection === 'asc' ? 'arrow-up' : 'arrow-down'}
                                    size={18}
                                    color={colors.textPrimary}
                                />
                            </TouchableOpacity>
                        </View>

                        <Text style={styles.exportSectionLabel}>Group</Text>
                        <View style={styles.exportSegmentedRow}>
                            <TouchableOpacity
                                style={[
                                    styles.exportToggleButton,
                                    exportGroupByAuthor && styles.exportOptionSelected
                                ]}
                                onPress={() => setExportGroupByAuthor(!exportGroupByAuthor)}
                            >
                                <Ionicons
                                    name={exportGroupByAuthor ? 'checkbox' : 'square-outline'}
                                    size={18}
                                    color={exportGroupByAuthor ? colors.accent : colors.textSecondary}
                                />
                                <Text style={styles.exportSegmentText}>Author</Text>
                            </TouchableOpacity>
                            <TouchableOpacity
                                style={[
                                    styles.exportToggleButton,
                                    exportGroupBySeries && styles.exportOptionSelected
                                ]}
                                onPress={() => setExportGroupBySeries(!exportGroupBySeries)}
                            >
                                <Ionicons
                                    name={exportGroupBySeries ? 'checkbox' : 'square-outline'}
                                    size={18}
                                    color={exportGroupBySeries ? colors.accent : colors.textSecondary}
                                />
                                <Text style={styles.exportSegmentText}>Series</Text>
                            </TouchableOpacity>
                        </View>

                        <TouchableOpacity
                            style={[
                                styles.exportActionButton,
                                (isExporting || isCopyingExportText || exportFormats.length === 0) && styles.exportActionButtonDisabled
                            ]}
                            onPress={handleExportLibrary}
                            disabled={isExporting || isCopyingExportText || exportFormats.length === 0}
                        >
                            {isExporting ? (
                                <ActivityIndicator size="small" color={colors.background} />
                            ) : (
                                <Ionicons name="download" size={18} color={colors.background} />
                            )}
                            <Text style={styles.exportActionText}>
                                {isExporting ? 'Exporting...' : 'Export'}
                            </Text>
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={[
                                styles.exportCopyButton,
                                (isExporting || isCopyingExportText) && styles.exportActionButtonDisabled
                            ]}
                            onPress={handleCopyExportText}
                            disabled={isExporting || isCopyingExportText}
                        >
                            {isCopyingExportText ? (
                                <ActivityIndicator size="small" color={colors.accent} />
                            ) : (
                                <Ionicons name="copy-outline" size={18} color={colors.accent} />
                            )}
                            <Text style={styles.exportCopyText}>
                                {isCopyingExportText ? 'Copying...' : 'Copy TXT'}
                            </Text>
                        </TouchableOpacity>
                    </View>
                )}

                <Text style={styles.headerSubtitle}>
                    {totalCount > 0 ? `${audiobooks.length} of ${totalCount} audiobooks` : `${audiobooks.length} audiobooks`}
                </Text>
            </View>

            {isLoading ? (
                <View style={styles.emptyState}>
                    <Text style={styles.emptyText}>Loading library...</Text>
                </View>
            ) : audiobooks.length === 0 ? (
                <View style={styles.emptyState}>
                    <Text style={styles.emptyText}>
                        {searchQuery || selectedSeries.length > 0 || selectedCategories.length > 0
                            ? 'No books match your search or filters'
                            : 'No audiobooks yet'}
                    </Text>
                    <Text style={styles.emptySubtext}>
                        {searchQuery || selectedSeries.length > 0 || selectedCategories.length > 0 || sourceFilter !== 'all'
                            ? 'Try adjusting your search or clearing filters'
                            : 'Go to Account tab to sign in and sync your Audible library, or browse free audiobooks on the Browse tab'}
                    </Text>
                </View>
            ) : (
                <FlatList
                    data={audiobooks}
                    renderItem={renderItem}
		    keyExtractor={(item, index) =>
		      `${item.id ?? item.audible_product_id}-${index}`
		    }                   
		    contentContainerStyle={styles.list}
                    ItemSeparatorComponent={() => <View style={styles.separator}/>}
                    onEndReached={handleLoadMore}
                    onEndReachedThreshold={0.5}
                    ListFooterComponent={
                        isLoadingMore ? (
                            <View style={styles.loadingFooter}>
                                <Text style={styles.loadingText}>Loading more...</Text>
                            </View>
                        ) : null
                    }
                    refreshControl={
                        <RefreshControl
                            refreshing={isRefreshing}
                            onRefresh={handleRefresh}
                            tintColor={colors.accent}
                            colors={[colors.accent]}
                        />
                    }
                />
            )}

            <Modal
                visible={!!selectedPodcast}
                animationType="slide"
                onRequestClose={handleClosePodcast}
            >
                <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
                    <View style={styles.episodeHeader}>
                        <TouchableOpacity
                            style={styles.episodeBackButton}
                            onPress={handleClosePodcast}
                            accessibilityLabel="Back to library"
                        >
                            <Ionicons name="arrow-back" size={24} color={colors.textPrimary} />
                        </TouchableOpacity>
                        <View style={styles.episodeHeaderText}>
                            <Text style={styles.episodeHeaderTitle} numberOfLines={1}>
                                {selectedPodcast?.title || 'Episodes'}
                            </Text>
                            <Text style={styles.episodeHeaderSubtitle}>
                                {podcastEpisodeCount > 0
                                    ? `${podcastEpisodes.length} of ${podcastEpisodeCount} episodes`
                                    : `${podcastEpisodes.length} episodes`}
                            </Text>
                        </View>
                        {podcastEpisodes.length > 0 && (
                            <TouchableOpacity
                                style={styles.episodeDownloadAllButton}
                                onPress={handleDownloadAllEpisodes}
                                accessibilityLabel="Download all episodes"
                            >
                                <Ionicons name="download-outline" size={24} color={colors.accent} />
                            </TouchableOpacity>
                        )}
                    </View>

                    {isLoadingPodcastEpisodes ? (
                        <View style={styles.emptyState}>
                            <ActivityIndicator size="large" color={colors.accent} />
                        </View>
                    ) : podcastEpisodes.length === 0 ? (
                        <View style={styles.emptyState}>
                            <Text style={styles.emptyText}>No episodes</Text>
                        </View>
                    ) : (
                        <FlatList
                            data={podcastEpisodes}
                            renderItem={renderPodcastEpisodeItem}
                            keyExtractor={(item, index) =>
                                `${item.id ?? item.audible_product_id}-${index}`
                            }
                            contentContainerStyle={styles.list}
                            ItemSeparatorComponent={() => <View style={styles.separator}/>}
                            onEndReached={handleLoadMorePodcastEpisodes}
                            onEndReachedThreshold={0.5}
                            ListFooterComponent={
                                isLoadingMorePodcastEpisodes ? (
                                    <View style={styles.loadingFooter}>
                                        <Text style={styles.loadingText}>Loading more...</Text>
                                    </View>
                                ) : null
                            }
                            refreshControl={
                                <RefreshControl
                                    refreshing={isRefreshingPodcastEpisodes}
                                    onRefresh={handlePodcastEpisodesRefresh}
                                    tintColor={colors.accent}
                                    colors={[colors.accent]}
                                />
                            }
                        />
                    )}
                </SafeAreaView>
            </Modal>

            {/* Sort Modal */}
            <Modal
                visible={showSortModal}
                transparent
                animationType="slide"
                onRequestClose={() => setShowSortModal(false)}
            >
                <TouchableOpacity
                    style={styles.modalOverlay}
                    activeOpacity={1}
                    onPress={() => setShowSortModal(false)}
                >
                    <View style={styles.modalContent} onStartShouldSetResponder={() => true}>
                        <Text style={styles.modalTitle}>Sort By</Text>

                        <TouchableOpacity
                            style={styles.modalOption}
                            onPress={() => handleSortChange('title', sortDirection === 'asc' ? 'desc' : 'asc')}
                        >
                            <Text style={styles.modalOptionText}>
                                Title {sortField === 'title' && (sortDirection === 'asc' ? '↑' : '↓')}
                            </Text>
                            {sortField === 'title' && <Text style={styles.modalCheck}>✓</Text>}
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={styles.modalOption}
                            onPress={() => handleSortChange('release_date', sortDirection === 'asc' ? 'desc' : 'asc')}
                        >
                            <Text style={styles.modalOptionText}>
                                Release Date {sortField === 'release_date' && (sortDirection === 'asc' ? '↑' : '↓')}
                            </Text>
                            {sortField === 'release_date' && <Text style={styles.modalCheck}>✓</Text>}
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={styles.modalOption}
                            onPress={() => handleSortChange('date_added', sortDirection === 'asc' ? 'desc' : 'asc')}
                        >
                            <Text style={styles.modalOptionText}>
                                Date Added {sortField === 'date_added' && (sortDirection === 'asc' ? '↑' : '↓')}
                            </Text>
                            {sortField === 'date_added' && <Text style={styles.modalCheck}>✓</Text>}
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={styles.modalOption}
                            onPress={() => handleSortChange('series', sortDirection === 'asc' ? 'desc' : 'asc')}
                        >
                            <Text style={styles.modalOptionText}>
                                Series {sortField === 'series' && (sortDirection === 'asc' ? '↑' : '↓')}
                            </Text>
                            {sortField === 'series' && <Text style={styles.modalCheck}>✓</Text>}
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={styles.modalOption}
                            onPress={() => handleSortChange('length', sortDirection === 'asc' ? 'desc' : 'asc')}
                        >
                            <Text style={styles.modalOptionText}>
                                Length {sortField === 'length' && (sortDirection === 'asc' ? '↑' : '↓')}
                            </Text>
                            {sortField === 'length' && <Text style={styles.modalCheck}>✓</Text>}
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={styles.modalOption}
                            onPress={() => handleSortChange('downloaded', sortField === 'downloaded' && sortDirection === 'desc' ? 'asc' : 'desc')}
                        >
                            <Text style={styles.modalOptionText}>
                                Downloaded {sortField === 'downloaded' && (sortDirection === 'asc' ? '↑' : '↓')}
                            </Text>
                            {sortField === 'downloaded' && <Text style={styles.modalCheck}>✓</Text>}
                        </TouchableOpacity>

                        <TouchableOpacity
                            style={styles.modalCancelButton}
                            onPress={() => setShowSortModal(false)}
                        >
                            <Text style={styles.modalCancelText}>Cancel</Text>
                        </TouchableOpacity>
                    </View>
                </TouchableOpacity>
            </Modal>

            {/* Filter Modal */}
            <Modal
                visible={showFilterModal}
                transparent
                animationType="slide"
                onRequestClose={() => setShowFilterModal(false)}
            >
                <TouchableOpacity
                    style={styles.modalOverlay}
                    activeOpacity={1}
                    onPress={() => setShowFilterModal(false)}
                >
                    <View style={styles.modalContentLarge} onStartShouldSetResponder={() => true}>
                        <View style={styles.modalHeader}>
                            <Text style={styles.modalTitle}>Filter</Text>
                            <TouchableOpacity onPress={handleClearFilters}>
                                <Text style={styles.clearFiltersText}>Clear All</Text>
                            </TouchableOpacity>
                        </View>

                        <ScrollView style={styles.filterScroll}>
                            {/* Source Filter */}
                            <Text style={styles.filterSectionTitle}>Source</Text>
                            {(['all', 'audible', 'librivox'] as SourceFilter[]).map((src) => (
                                <TouchableOpacity
                                    key={src}
                                    style={[
                                        styles.filterOption,
                                        sourceFilter === src && styles.filterOptionSelected
                                    ]}
                                    onPress={() => setSourceFilter(src)}
                                >
                                    <Text style={styles.filterOptionText}>
                                        {src === 'all' ? 'All Sources' : src === 'audible' ? 'Audible' : 'LibriVox'}
                                    </Text>
                                    {sourceFilter === src && <Text style={styles.modalCheck}>✓</Text>}
                                </TouchableOpacity>
                            ))}

                            {/* Type Filter */}
                            <Text style={styles.filterSectionTitle}>Type</Text>
                            {([
                                {value: 'all', label: 'All Types'},
                                {value: 'audiobooks', label: 'Audiobooks'},
                                {value: 'podcasts', label: 'Podcasts'},
                            ] as const).map((option) => (
                                <TouchableOpacity
                                    key={option.value}
                                    style={[
                                        styles.filterOption,
                                        typeFilter === option.value && styles.filterOptionSelected
                                    ]}
                                    onPress={() => setTypeFilter(option.value)}
                                >
                                    <Text style={styles.filterOptionText}>{option.label}</Text>
                                    {typeFilter === option.value && <Text style={styles.modalCheck}>✓</Text>}
                                </TouchableOpacity>
                            ))}

                            {allAccounts.length > 1 &&
                                renderMultiSelectSection(
                                    'Account',
                                    'All Accounts',
                                    allAccounts.map((savedAccount) => savedAccount.account_id),
                                    accountFilters,
                                    setAccountFilters,
                                    'accounts',
                                    (accountId) => {
                                        const savedAccount = allAccounts.find((a) => a.account_id === accountId);
                                        return savedAccount?.account_name || accountId;
                                    }
                                )}

                            {/* Genre Filter */}
                            {renderMultiSelectSection(
                                'Genre',
                                'All Genres',
                                allCategories,
                                selectedCategories,
                                setSelectedCategories,
                                'genres'
                            )}

                            {/* Series Filter */}
                            {renderMultiSelectSection(
                                'Series',
                                'All Series',
                                allSeries,
                                selectedSeries,
                                setSelectedSeries,
                                'series'
                            )}
                        </ScrollView>

                        <TouchableOpacity
                            style={styles.modalApplyButton}
                            onPress={() => setShowFilterModal(false)}
                        >
                            <Text style={styles.modalApplyText}>Apply Filters</Text>
                        </TouchableOpacity>
                    </View>
                </TouchableOpacity>
            </Modal>

            {/* Book Detail Modal */}
            <Modal
                visible={showDetailModal}
                transparent
                animationType="slide"
                onRequestClose={() => setShowDetailModal(false)}
            >
                <TouchableOpacity
                    style={styles.modalOverlay}
                    activeOpacity={1}
                    onPress={() => setShowDetailModal(false)}
                >
                    <View style={styles.modalContentLarge} onStartShouldSetResponder={() => true}>
                        <Text style={styles.modalTitle}>{selectedBook?.title}</Text>
                        {!!selectedBook?.subtitle && (
                            <Text style={styles.modalSubtitle}>{selectedBook.subtitle}</Text>
                        )}
                        <ScrollView style={styles.filterScroll}>
                            {(selectedBook?.authors?.length || 0) > 0 && (
                                <Text style={styles.detailLine}>By {selectedBook!.authors.join(', ')}</Text>
                            )}
                            {(selectedBook?.narrators?.length || 0) > 0 && (
                                <Text style={styles.detailLine}>Narrated by {selectedBook!.narrators.join(', ')}</Text>
                            )}
                            {!!selectedBook?.series_name && (
                                <Text style={styles.detailLine}>
                                    {selectedBook.series_name}
                                    {selectedBook.series_sequence ? ` #${selectedBook.series_sequence}` : ''}
                                </Text>
                            )}
                            {!!selectedBook?.duration_seconds && (
                                <Text style={styles.detailLine}>{formatDuration(selectedBook.duration_seconds)}</Text>
                            )}
                            {!!selectedBook?.release_date && (
                                <Text style={styles.detailLine}>Released {selectedBook.release_date.split('T')[0]}</Text>
                            )}
                            {allAccounts.length > 1 && selectedBook && getBookAccountLabels(selectedBook).length > 0 && (
                                <Text style={styles.detailLine}>
                                    Account: {getBookAccountLabels(selectedBook).join(', ')}
                                </Text>
                            )}
                            <Text
                                style={styles.detailDescription}
                                numberOfLines={detailDescriptionExpanded ? undefined : DETAIL_DESCRIPTION_LINES}
                            >
                                {stripHtml(selectedBook?.description) || 'No summary available.'}
                            </Text>
                            {/* Invisible unclamped copy measures the real line count */}
                            {detailDescriptionLines === 0 && (
                                <Text
                                    style={[styles.detailDescription, styles.detailDescriptionMeasure]}
                                    onTextLayout={(e) =>
                                        setDetailDescriptionLines(e.nativeEvent.lines.length)
                                    }
                                >
                                    {stripHtml(selectedBook?.description) || 'No summary available.'}
                                </Text>
                            )}
                            {detailDescriptionLines > DETAIL_DESCRIPTION_LINES && (
                                <TouchableOpacity
                                    onPress={() => setDetailDescriptionExpanded((prev) => !prev)}
                                >
                                    <Text style={[styles.detailLine, {color: colors.accent}]}>
                                        {detailDescriptionExpanded ? 'Show less' : 'Show more'}
                                    </Text>
                                </TouchableOpacity>
                            )}
                        </ScrollView>
                        {selectedBook && isPodcastParent(selectedBook) && (
                            <TouchableOpacity
                                style={styles.modalApplyButton}
                                onPress={() => {
                                    setShowDetailModal(false);
                                    handlePodcastPress(selectedBook);
                                }}
                            >
                                <Text style={styles.modalApplyText}>View Episodes</Text>
                            </TouchableOpacity>
                        )}
                        <TouchableOpacity
                            style={styles.modalApplyButton}
                            onPress={() => setShowDetailModal(false)}
                        >
                            <Text style={styles.modalApplyText}>Close</Text>
                        </TouchableOpacity>
                    </View>
                </TouchableOpacity>
            </Modal>

            {/* Context Menu Modal */}
            <Modal
                visible={showContextMenu}
                transparent
                animationType="slide"
                onRequestClose={() => setShowContextMenu(false)}
            >
                <TouchableOpacity
                    style={styles.modalOverlay}
                    activeOpacity={1}
                    onPress={() => setShowContextMenu(false)}
                >
                    <View style={styles.modalContent} onStartShouldSetResponder={() => true}>
                        <Text style={styles.modalTitle}>
                            {selectedBook?.title || 'Book Options'}
                        </Text>
                        <Text style={styles.modalSubtitle}>
                            {selectedBook?.authors?.join(', ') || ''}
                        </Text>

                        {!selectedBookIsPodcastParent && (
                            <>
                                <TouchableOpacity
                                    style={styles.modalOption}
                                    onPress={() => {
                                        setShowContextMenu(false);
                                        if (selectedBook) {
                                            handleSelectFileAsDownloaded(selectedBook);
                                        }
                                    }}
                                >
                                    <Ionicons
                                        name="document-attach"
                                        size={24}
                                        color={colors.accent}
                                        style={styles.modalOptionIcon}
                                    />
                                    <View style={styles.modalOptionTextContainer}>
                                        <Text style={styles.modalOptionText}>Select File as Downloaded</Text>
                                        <Text style={styles.modalOptionDescription}>
                                            Choose an existing audio file on your device
                                        </Text>
                                    </View>
                                </TouchableOpacity>

                                <TouchableOpacity
                                    style={styles.modalOption}
                                    onPress={() => {
                                        setShowContextMenu(false);
                                        if (selectedBook) {
                                            handleCreateCoverArt(selectedBook);
                                        }
                                    }}
                                >
                                    <Ionicons
                                        name="image"
                                        size={24}
                                        color={colors.accent}
                                        style={styles.modalOptionIcon}
                                    />
                                    <View style={styles.modalOptionTextContainer}>
                                        <Text style={styles.modalOptionText}>Create Cover Art File</Text>
                                        <Text style={styles.modalOptionDescription}>
                                            Save EmbeddedCover.jpg for Smart Audiobook Player
                                        </Text>
                                    </View>
                                </TouchableOpacity>
                            </>
                        )}

                        {selectedBook && !selectedBookIsPodcastParent && (
                            <>
                                <View style={styles.modalDivider} />
                                <TouchableOpacity
                                    style={styles.modalOption}
                                    onPress={() => {
                                        setShowContextMenu(false);
                                        handleMarkAsNotDownloaded(selectedBook);
                                    }}
                                >
                                    <Ionicons
                                        name="trash-outline"
                                        size={24}
                                        color={colors.error}
                                        style={styles.modalOptionIcon}
                                    />
                                    <View style={styles.modalOptionTextContainer}>
                                        <Text style={[styles.modalOptionText, {color: colors.error}]}>
                                            Mark as Not Downloaded
                                        </Text>
                                        <Text style={styles.modalOptionDescription}>
                                            Clear download status and optionally delete file
                                        </Text>
                                    </View>
                                </TouchableOpacity>
                            </>
                        )}

                        <TouchableOpacity
                            style={styles.modalCancelButton}
                            onPress={() => setShowContextMenu(false)}
                        >
                            <Text style={styles.modalCancelText}>Cancel</Text>
                        </TouchableOpacity>
                    </View>
                </TouchableOpacity>
            </Modal>
        </SafeAreaView>
    );
}

const createStyles = (theme: Theme) => ({
    container: {
        flex: 1,
        backgroundColor: theme.colors.background,
    },
    header: {
        padding: theme.spacing.lg,
        borderBottomWidth: 1,
        borderBottomColor: theme.colors.border,
    },
    headerTitleRow: {
        flexDirection: 'row' as const,
        justifyContent: 'space-between' as const,
        alignItems: 'center' as const,
        marginBottom: theme.spacing.md,
    },
    headerTitle: {
        ...theme.typography.title,
    },
    headerActions: {
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        gap: theme.spacing.sm,
    },
    toggleControlsButton: {
        padding: theme.spacing.xs,
        paddingHorizontal: theme.spacing.sm,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: 'transparent',
    },
    toggleControlsButtonActive: {
        borderColor: theme.colors.accent,
        backgroundColor: theme.colors.accent + '20',
    },
    searchContainer: {
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        backgroundColor: theme.colors.backgroundSecondary,
        borderRadius: 8,
        paddingHorizontal: theme.spacing.md,
        marginBottom: theme.spacing.md,
        borderWidth: 1,
        borderColor: theme.colors.border,
    },
    searchIcon: {
        marginRight: theme.spacing.sm,
    },
    searchInput: {
        flex: 1,
        ...theme.typography.body,
        color: theme.colors.textPrimary,
        paddingVertical: theme.spacing.sm,
    },
    controlsRow: {
        flexDirection: 'row' as const,
        gap: theme.spacing.sm,
        marginBottom: theme.spacing.sm,
    },
    controlButton: {
        flex: 1,
        backgroundColor: theme.colors.backgroundSecondary,
        borderRadius: 8,
        paddingVertical: theme.spacing.sm,
        paddingHorizontal: theme.spacing.md,
        borderWidth: 1,
        borderColor: theme.colors.border,
        alignItems: 'center' as const,
    },
    controlButtonActive: {
        borderColor: theme.colors.accent,
        backgroundColor: theme.colors.accent + '20',
    },
    controlButtonText: {
        ...theme.typography.caption,
        fontWeight: '600' as const,
    },
    exportContainer: {
        backgroundColor: theme.colors.backgroundSecondary,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: theme.colors.border,
        padding: theme.spacing.md,
        marginBottom: theme.spacing.md,
        gap: theme.spacing.sm,
    },
    exportSectionLabel: {
        ...theme.typography.caption,
        fontWeight: '700' as const,
        color: theme.colors.textPrimary,
    },
    exportFormatGrid: {
        flexDirection: 'row' as const,
        flexWrap: 'wrap' as const,
        gap: theme.spacing.sm,
    },
    exportFormatButton: {
        flexBasis: '48%' as const,
        flexGrow: 1,
        minHeight: 42,
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        justifyContent: 'center' as const,
        gap: theme.spacing.xs,
        backgroundColor: theme.colors.background,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: theme.colors.border,
        paddingHorizontal: theme.spacing.sm,
    },
    exportFormatText: {
        ...theme.typography.caption,
        fontWeight: '700' as const,
        color: theme.colors.textPrimary,
    },
    exportSegmentedRow: {
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        gap: theme.spacing.sm,
    },
    exportSegmentButton: {
        flex: 1,
        minHeight: 40,
        alignItems: 'center' as const,
        justifyContent: 'center' as const,
        backgroundColor: theme.colors.background,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: theme.colors.border,
        paddingHorizontal: theme.spacing.sm,
    },
    exportDirectionButton: {
        width: 44,
        height: 40,
        alignItems: 'center' as const,
        justifyContent: 'center' as const,
        backgroundColor: theme.colors.background,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: theme.colors.border,
    },
    exportToggleButton: {
        flex: 1,
        minHeight: 40,
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        justifyContent: 'center' as const,
        gap: theme.spacing.xs,
        backgroundColor: theme.colors.background,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: theme.colors.border,
        paddingHorizontal: theme.spacing.sm,
    },
    exportOptionSelected: {
        borderColor: theme.colors.accent,
        backgroundColor: theme.colors.accent + '20',
    },
    exportSegmentText: {
        ...theme.typography.caption,
        fontWeight: '700' as const,
        color: theme.colors.textPrimary,
    },
    exportActionButton: {
        minHeight: 44,
        marginTop: theme.spacing.xs,
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        justifyContent: 'center' as const,
        gap: theme.spacing.sm,
        backgroundColor: theme.colors.accent,
        borderRadius: 8,
        paddingHorizontal: theme.spacing.md,
    },
    exportActionButtonDisabled: {
        opacity: 0.6,
    },
    exportActionText: {
        ...theme.typography.body,
        fontWeight: '700' as const,
        color: theme.colors.background,
    },
    exportCopyButton: {
        minHeight: 44,
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        justifyContent: 'center' as const,
        gap: theme.spacing.sm,
        backgroundColor: theme.colors.background,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: theme.colors.accent,
        paddingHorizontal: theme.spacing.md,
    },
    exportCopyText: {
        ...theme.typography.body,
        fontWeight: '700' as const,
        color: theme.colors.accent,
    },
    headerSubtitle: {
        ...theme.typography.caption,
    },
    list: {
        padding: theme.spacing.md,
    },
    item: {
        backgroundColor: theme.colors.backgroundSecondary,
        borderRadius: 8,
        padding: theme.spacing.md,
        borderWidth: 1,
        borderColor: theme.colors.border,
    },
    itemSelected: {
        borderColor: theme.colors.accent,
        backgroundColor: theme.colors.accentDim,
    },
    batchCheck: {
        width: 24,
        height: 24,
        borderRadius: 12,
        borderWidth: 2,
        borderColor: theme.colors.border,
        alignSelf: 'center' as const,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
    },
    batchCheckOn: {
        backgroundColor: theme.colors.accent,
        borderColor: theme.colors.accent,
    },
    batchCheckDisabled: {
        opacity: 0.3,
    },
    batchCheckMark: {
        color: theme.colors.background,
        fontWeight: '700' as const,
        fontSize: 14,
    },
    batchBar: {
        flexDirection: 'row' as const,
        justifyContent: 'space-between' as const,
        alignItems: 'center' as const,
        paddingHorizontal: theme.spacing.md,
        paddingVertical: theme.spacing.sm,
        backgroundColor: theme.colors.backgroundSecondary,
        borderBottomWidth: 1,
        borderBottomColor: theme.colors.border,
    },
    batchBarText: {
        ...theme.typography.body,
        fontWeight: '600' as const,
    },
    batchBarButtons: {
        flexDirection: 'row' as const,
        gap: theme.spacing.sm,
    },
    batchBarButtonSecondary: {
        paddingHorizontal: theme.spacing.md,
        paddingVertical: theme.spacing.sm,
        borderRadius: 6,
        borderWidth: 1,
        borderColor: theme.colors.border,
    },
    batchBarButtonSecondaryText: {
        ...theme.typography.body,
        fontSize: 14,
    },
    batchBarButton: {
        paddingHorizontal: theme.spacing.md,
        paddingVertical: theme.spacing.sm,
        borderRadius: 6,
        backgroundColor: theme.colors.accent,
    },
    batchBarButtonDisabled: {
        opacity: 0.4,
    },
    batchBarButtonText: {
        ...theme.typography.body,
        fontSize: 14,
        color: theme.colors.background,
        fontWeight: '700' as const,
    },
    itemRow: {
        flexDirection: 'row' as const,
        gap: theme.spacing.md,
    },
    cover: {
        width: 80,
        height: 80,
        borderRadius: 4,
        backgroundColor: theme.colors.background,
    },
    coverPlaceholder: {
        width: 80,
        height: 80,
        borderRadius: 4,
        backgroundColor: theme.colors.background,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
    },
    coverPlaceholderText: {
        fontSize: 32,
    },
    itemContent: {
        flex: 1,
        gap: theme.spacing.xs,
    },
    title: {
        ...theme.typography.subtitle,
        fontSize: 16,
    },
    author: {
        ...theme.typography.caption,
    },
    series: {
        ...theme.typography.caption,
        color: theme.colors.accent,
        fontStyle: 'italic' as const,
    },
    metadata: {
        flexDirection: 'row' as const,
        justifyContent: 'space-between' as const,
        alignItems: 'center' as const,
        marginTop: theme.spacing.xs,
    },
    duration: {
        ...theme.typography.caption,
        fontFamily: 'monospace',
    },
    status: {
        ...theme.typography.caption,
        fontWeight: '600' as const,
    },
    sourceBadge: {
        backgroundColor: theme.colors.accentSecondary + '30',
        paddingHorizontal: 6,
        paddingVertical: 2,
        borderRadius: 4,
    },
    sourceBadgeText: {
        fontSize: 10,
        fontWeight: '600' as const,
        color: theme.colors.accentSecondary,
    },
    separator: {
        height: theme.spacing.sm,
    },
    detailLine: {
        fontSize: 14,
        color: theme.colors.textSecondary,
        marginBottom: theme.spacing.xs,
    },
    detailDescription: {
        fontSize: 14,
        color: theme.colors.textPrimary,
        lineHeight: 20,
        marginTop: theme.spacing.sm,
        marginBottom: theme.spacing.md,
    },
    detailDescriptionMeasure: {
        position: 'absolute' as const,
        left: 0,
        right: 0,
        opacity: 0,
        zIndex: -1,
    },
    emptyState: {
        flex: 1,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
        padding: theme.spacing.xl,
    },
    emptyText: {
        ...theme.typography.subtitle,
        marginBottom: theme.spacing.sm,
        textAlign: 'center' as const,
    },
    emptySubtext: {
        ...theme.typography.caption,
        textAlign: 'center' as const,
    },
    loadingFooter: {
        padding: theme.spacing.md,
        alignItems: 'center' as const,
    },
    loadingText: {
        ...theme.typography.caption,
        color: theme.colors.textSecondary,
    },
    downloadButton: {
        width: 44,
        height: 44,
        borderRadius: 22,
        backgroundColor: theme.colors.accent,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
    },
    downloadButtonText: {
        fontSize: 20,
        color: theme.colors.background,
    },
    episodeButton: {
        width: 44,
        height: 44,
        borderRadius: 22,
        backgroundColor: theme.colors.background,
        borderWidth: 1,
        borderColor: theme.colors.accent,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
    },
    pauseButton: {
        width: 44,
        height: 44,
        borderRadius: 22,
        backgroundColor: theme.colors.warning,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
        marginRight: theme.spacing.xs,
    },
    pauseButtonText: {
        fontSize: 18,
        color: theme.colors.background,
    },
    resumeButton: {
        width: 44,
        height: 44,
        borderRadius: 22,
        backgroundColor: theme.colors.success,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
        marginRight: theme.spacing.xs,
    },
    resumeButtonText: {
        fontSize: 18,
        color: theme.colors.background,
    },
    cancelButton: {
        width: 44,
        height: 44,
        borderRadius: 22,
        backgroundColor: theme.colors.error,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
    },
    cancelButtonText: {
        fontSize: 20,
        color: theme.colors.background,
    },
    episodeHeader: {
        flexDirection: 'row' as const,
        alignItems: 'center' as const,
        gap: theme.spacing.md,
        padding: theme.spacing.lg,
        borderBottomWidth: 1,
        borderBottomColor: theme.colors.border,
        backgroundColor: theme.colors.backgroundSecondary,
    },
    episodeBackButton: {
        width: 44,
        height: 44,
        borderRadius: 22,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
        backgroundColor: theme.colors.background,
        borderWidth: 1,
        borderColor: theme.colors.border,
    },
    episodeHeaderText: {
        flex: 1,
        minWidth: 0,
    },
    episodeDownloadAllButton: {
        width: 44,
        height: 44,
        borderRadius: 22,
        justifyContent: 'center' as const,
        alignItems: 'center' as const,
        backgroundColor: theme.colors.background,
        borderWidth: 1,
        borderColor: theme.colors.border,
    },
    episodeHeaderTitle: {
        ...theme.typography.subtitle,
        color: theme.colors.textPrimary,
    },
    episodeHeaderSubtitle: {
        ...theme.typography.caption,
        color: theme.colors.textSecondary,
    },
    modalOverlay: {
        flex: 1,
        backgroundColor: 'rgba(0, 0, 0, 0.5)',
        justifyContent: 'flex-end' as const,
    },
    modalContent: {
        backgroundColor: theme.colors.backgroundSecondary,
        borderTopLeftRadius: 20,
        borderTopRightRadius: 20,
        padding: theme.spacing.lg,
        paddingBottom: theme.spacing.xl,
    },
    modalContentLarge: {
        backgroundColor: theme.colors.backgroundSecondary,
        borderTopLeftRadius: 20,
        borderTopRightRadius: 20,
        padding: theme.spacing.lg,
        paddingBottom: theme.spacing.xl,
    },
    modalHeader: {
        flexDirection: 'row' as const,
        justifyContent: 'space-between' as const,
        alignItems: 'center' as const,
        marginBottom: theme.spacing.md,
    },
    modalTitle: {
        ...theme.typography.title,
        fontSize: 20,
    },
    clearFiltersText: {
        ...theme.typography.body,
        color: theme.colors.accent,
    },
    modalOption: {
        flexDirection: 'row' as const,
        justifyContent: 'space-between' as const,
        alignItems: 'center' as const,
        paddingVertical: theme.spacing.md,
        borderBottomWidth: 1,
        borderBottomColor: theme.colors.border,
    },
    modalOptionText: {
        ...theme.typography.body,
    },
    modalCheck: {
        ...theme.typography.body,
        color: theme.colors.accent,
        fontSize: 20,
    },
    modalCancelButton: {
        marginTop: theme.spacing.lg,
        padding: theme.spacing.md,
        backgroundColor: theme.colors.background,
        borderRadius: 8,
        alignItems: 'center' as const,
    },
    modalCancelText: {
        ...theme.typography.body,
        fontWeight: '600' as const,
    },
    filterScroll: {
        maxHeight: 400,
    },
    filterSectionTitle: {
        ...theme.typography.subtitle,
        marginTop: theme.spacing.lg,
        marginBottom: theme.spacing.sm,
        color: theme.colors.accent,
    },
    filterOption: {
        flexDirection: 'row' as const,
        justifyContent: 'space-between' as const,
        alignItems: 'center' as const,
        paddingVertical: theme.spacing.sm,
        paddingHorizontal: theme.spacing.md,
        borderRadius: 8,
        marginBottom: theme.spacing.xs,
    },
    filterOptionSelected: {
        backgroundColor: theme.colors.accent + '20',
    },
    filterOptionText: {
        ...theme.typography.body,
    },
    modalApplyButton: {
        marginTop: theme.spacing.lg,
        padding: theme.spacing.md,
        backgroundColor: theme.colors.accent,
        borderRadius: 8,
        alignItems: 'center' as const,
    },
    modalApplyText: {
        ...theme.typography.body,
        fontWeight: '600' as const,
        color: theme.colors.background,
    },
    modalSubtitle: {
        ...theme.typography.caption,
        marginBottom: theme.spacing.md,
        color: theme.colors.textSecondary,
    },
    modalOptionIcon: {
        marginRight: theme.spacing.md,
    },
    modalOptionTextContainer: {
        flex: 1,
    },
    modalOptionDescription: {
        ...theme.typography.caption,
        color: theme.colors.textSecondary,
        marginTop: theme.spacing.xs,
    },
    modalDivider: {
        height: 1,
        backgroundColor: theme.colors.border,
        marginVertical: theme.spacing.sm,
    },
});
