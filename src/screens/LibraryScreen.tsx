import React, {useState, useEffect, useRef} from 'react';
import {View, Text, FlatList, TouchableOpacity, RefreshControl, Image, Alert, ActivityIndicator, Platform, PermissionsAndroid, TextInput, Modal, ScrollView} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {useFocusEffect} from '@react-navigation/native';
import {Ionicons} from '@expo/vector-icons';
import {useStyles} from '../hooks/useStyles';
import {useTheme} from '../styles/theme';
import type {Theme} from '../hooks/useStyles';
import {
    getBooks,
    getBooksWithFilters,
    getAllSeries,
    getAllCategories,
    initializeDatabase,
    refreshToken,
    enqueueDownloadNew,
    listDownloadTasks,
    pauseDownload,
    resumeDownload,
    cancelDownload,
} from '../../modules/expo-rust-bridge';
import type {Book, Account, DownloadTask} from '../../modules/expo-rust-bridge';
import {Paths} from 'expo-file-system';
import * as SecureStore from 'expo-secure-store';

const DOWNLOAD_PATH_KEY = 'download_path';
const LIBRARY_PREFS_KEY = 'library_preferences';

type SortField = 'title' | 'release_date' | 'date_added' | 'series';
type SortDirection = 'asc' | 'desc';

interface LibraryPreferences {
    sortField: SortField;
    sortDirection: SortDirection;
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
    const progressInterval = useRef<NodeJS.Timeout | null>(null);

    // Search, filter, and sort state
    const [searchQuery, setSearchQuery] = useState('');
    const [sortField, setSortField] = useState<SortField>('title');
    const [sortDirection, setSortDirection] = useState<SortDirection>('asc');
    const [selectedSeries, setSelectedSeries] = useState<string | null>(null);
    const [selectedCategory, setSelectedCategory] = useState<string | null>(null);

    // Filter options
    const [allSeries, setAllSeries] = useState<string[]>([]);
    const [allCategories, setAllCategories] = useState<string[]>([]);

    // Modal state
    const [showFilterModal, setShowFilterModal] = useState(false);
    const [showSortModal, setShowSortModal] = useState(false);

    // Controls visibility
    const [showControls, setShowControls] = useState(false);

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
    }, [searchQuery, sortField, sortDirection, selectedSeries, selectedCategory]);

    // Reload books when tab is focused
    useFocusEffect(
        React.useCallback(() => {
            console.log('[LibraryScreen] Tab focused, reloading books...');
            loadBooks(true);
        }, [])
    );

    // Poll for download progress
    useEffect(() => {
        const pollProgress = () => {
            try {
                const cacheUri = Paths.cache.uri;
                const cachePath = cacheUri.replace('file://', '');
                const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

                const tasks = listDownloadTasks(dbPath);
                const taskMap = new Map<string, DownloadTask>();

                tasks.forEach(task => {
                    taskMap.set(task.asin, task);
                });

                setDownloadTasks(taskMap);
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
            }
        } catch (error) {
            console.error('[LibraryScreen] Error loading preferences:', error);
        }
    };

    const savePreferences = async (field: SortField, direction: SortDirection) => {
        try {
            const prefs: LibraryPreferences = {
                sortField: field,
                sortDirection: direction,
            };
            await SecureStore.setItemAsync(LIBRARY_PREFS_KEY, JSON.stringify(prefs));
        } catch (error) {
            console.error('[LibraryScreen] Error saving preferences:', error);
        }
    };

    const loadFilterOptions = async () => {
        try {
            const cacheUri = Paths.cache.uri;
            const cachePath = cacheUri.replace('file://', '');
            const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

            try {
                initializeDatabase(dbPath);
                const series = getAllSeries(dbPath);
                const categories = getAllCategories(dbPath);

                setAllSeries(series);
                setAllCategories(categories);
            } catch (error) {
                console.log('[LibraryScreen] Database not ready yet');
            }
        } catch (error) {
            console.error('[LibraryScreen] Error loading filter options:', error);
        }
    };

    const loadBooks = async (reset: boolean = false) => {
        try {
            const cacheUri = Paths.cache.uri;
            const cachePath = cacheUri.replace('file://', '');
            const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

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

            const offset = reset ? 0 : audiobooks.length;
            const limit = 100;

            console.log('[LibraryScreen] Fetching books:', {
                offset,
                limit,
                searchQuery,
                sortField,
                sortDirection,
                selectedSeries,
                selectedCategory,
            });

            const response = getBooksWithFilters(
                dbPath,
                offset,
                limit,
                searchQuery || null,
                selectedSeries || null,
                selectedCategory || null,
                sortField,
                sortDirection
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

    const handleSortChange = (field: SortField, direction: SortDirection) => {
        setSortField(field);
        setSortDirection(direction);
        savePreferences(field, direction);
        setShowSortModal(false);
    };

    const handleClearFilters = () => {
        setSearchQuery('');
        setSelectedSeries(null);
        setSelectedCategory(null);
        setShowFilterModal(false);
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

    const getStatus = (book: Book): { text: string; color: string } => {
        const task = downloadTasks.get(book.audible_product_id);

        if (task) {
            const percentage = task.total_bytes > 0
                ? ((task.bytes_downloaded / task.total_bytes) * 100).toFixed(1)
                : '0.0';

            switch (task.status) {
                case 'queued':
                    return {text: '⏳ Queued', color: colors.textSecondary};
                case 'downloading':
                    return {text: `⬇ ${percentage}%`, color: colors.info};
                case 'paused':
                    return {text: `⏸ Paused ${percentage}%`, color: colors.warning};
                case 'completed':
                    return {text: '✓ Downloaded', color: colors.success};
                case 'failed':
                    return {text: '✗ Failed', color: colors.error};
                case 'cancelled':
                    return {text: 'Cancelled', color: colors.textSecondary};
                default:
                    return {text: 'Available', color: colors.textSecondary};
            }
        }

        if (book.file_path) {
            return {text: '✓ Downloaded', color: colors.success};
        }

        return {text: 'Available', color: colors.textSecondary};
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

    const handleDownload = async (book: Book) => {
        try {
            const hasPermission = await requestNotificationPermission();
            if (!hasPermission) {
                Alert.alert(
                    'Permission Required',
                    'Please grant notification permission to see download progress',
                    [{ text: 'OK' }]
                );
                return;
            }

            const accountData = await SecureStore.getItemAsync('audible_account');
            if (!accountData) {
                Alert.alert('Error', 'Please log in first');
                return;
            }

            let account: Account = JSON.parse(accountData);

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

                        await SecureStore.setItemAsync('audible_account', JSON.stringify(account));
                        console.log('[LibraryScreen] Token refreshed successfully');
                    } catch (refreshError) {
                        console.error('[LibraryScreen] Token refresh failed:', refreshError);
                        Alert.alert('Error', 'Please log in again - token refresh failed');
                        return;
                    }
                }
            }

            const downloadDir = await SecureStore.getItemAsync(DOWNLOAD_PATH_KEY);

            if (!downloadDir) {
                Alert.alert(
                    'Download Directory Not Set',
                    'Please go to Settings and choose a download directory first.',
                    [{ text: 'OK' }]
                );
                return;
            }

            console.log('[LibraryScreen] Enqueueing download:', book.title, book.audible_product_id);

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

            Alert.alert(
                'Download Started',
                `${book.title} has been added to the download queue. You can monitor progress here or leave the app.`
            );

        } catch (error: any) {
            console.error('[LibraryScreen] Download error:', error);
            Alert.alert('Download Failed', error.message || 'Unknown error');
        }
    };

    const handlePauseDownload = (book: Book) => {
        try {
            const cacheUri = Paths.cache.uri;
            const cachePath = cacheUri.replace('file://', '');
            const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

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
            const cacheUri = Paths.cache.uri;
            const cachePath = cacheUri.replace('file://', '');
            const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

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
            const cacheUri = Paths.cache.uri;
            const cachePath = cacheUri.replace('file://', '');
            const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

            const task = downloadTasks.get(book.audible_product_id);
            if (task) {
                Alert.alert(
                    'Cancel Download',
                    `Are you sure you want to cancel downloading "${book.title}"?`,
                    [
                        { text: 'No', style: 'cancel' },
                        {
                            text: 'Yes',
                            style: 'destructive',
                            onPress: () => {
                                cancelDownload(dbPath, task.task_id);
                                console.log('[LibraryScreen] Cancelled download:', book.title);
                            }
                        }
                    ]
                );
            }
        } catch (error) {
            console.error('[LibraryScreen] Cancel error:', error);
        }
    };

    const renderItem = ({item}: { item: Book }) => {
        const status = getStatus(item);
        const authorText = (item.authors?.length || 0) > 0 ? item.authors.join(', ') : 'Unknown Author';
        const coverUrl = getCoverUrl(item);
        const task = downloadTasks.get(item.audible_product_id);
        const isDownloaded = !!item.file_path || task?.status === 'completed';
        const canDownload = !task || task.status === 'failed' || task.status === 'cancelled';
        const isDownloading = task?.status === 'downloading';
        const isPaused = task?.status === 'paused';
        const isQueued = task?.status === 'queued';

        return (
            <TouchableOpacity style={styles.item} onPress={() => console.log('Item pressed:', item)}>
                <View style={styles.itemRow}>
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
                        <View style={styles.metadata}>
                            <Text style={styles.duration}>{formatDuration(item.duration_seconds)}</Text>
                            <Text style={[styles.status, {color: status.color}]}>
                                {status.text}
                            </Text>
                        </View>
                    </View>

                    {!isDownloaded && canDownload && (
                        <TouchableOpacity
                            style={styles.downloadButton}
                            onPress={() => handleDownload(item)}
                        >
                            <Text style={styles.downloadButtonText}>⬇</Text>
                        </TouchableOpacity>
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

                    {(isDownloading || isPaused || isQueued) && (
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
                </View>
            </TouchableOpacity>
        );
    };

    const getSortLabel = () => {
        const fieldLabels = {
            title: 'Title',
            release_date: 'Release Date',
            date_added: 'Date Added',
            series: 'Series',
        };
        const arrow = sortDirection === 'asc' ? '↑' : '↓';
        return `${fieldLabels[sortField]} ${arrow}`;
    };

    const getActiveFiltersCount = () => {
        let count = 0;
        if (selectedSeries) count++;
        if (selectedCategory) count++;
        return count;
    };

    return (
        <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
            <View style={styles.header}>
                <View style={styles.headerTitleRow}>
                    <Text style={styles.headerTitle}>Library</Text>
                    <TouchableOpacity
                        style={styles.toggleControlsButton}
                        onPress={() => setShowControls(!showControls)}
                    >
                        <Ionicons
                            name={showControls ? 'close' : 'search'}
                            size={24}
                            color={colors.textPrimary}
                        />
                    </TouchableOpacity>
                </View>

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
                                placeholder="Search titles, authors, narrators..."
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
                        {searchQuery || selectedSeries || selectedCategory
                            ? 'No books match your search or filters'
                            : 'No audiobooks yet'}
                    </Text>
                    <Text style={styles.emptySubtext}>
                        {searchQuery || selectedSeries || selectedCategory
                            ? 'Try adjusting your search or clearing filters'
                            : 'Go to Account tab to sign in and sync your Audible library'}
                    </Text>
                </View>
            ) : (
                <FlatList
                    data={audiobooks}
                    renderItem={renderItem}
                    keyExtractor={(item) => item.audible_product_id}
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
                            {/* Series Filter */}
                            <Text style={styles.filterSectionTitle}>Series</Text>
                            <TouchableOpacity
                                style={[
                                    styles.filterOption,
                                    !selectedSeries && styles.filterOptionSelected
                                ]}
                                onPress={() => setSelectedSeries(null)}
                            >
                                <Text style={styles.filterOptionText}>All Series</Text>
                                {!selectedSeries && <Text style={styles.modalCheck}>✓</Text>}
                            </TouchableOpacity>

                            {allSeries.map((series) => (
                                <TouchableOpacity
                                    key={series}
                                    style={[
                                        styles.filterOption,
                                        selectedSeries === series && styles.filterOptionSelected
                                    ]}
                                    onPress={() => setSelectedSeries(series)}
                                >
                                    <Text style={styles.filterOptionText}>{series}</Text>
                                    {selectedSeries === series && <Text style={styles.modalCheck}>✓</Text>}
                                </TouchableOpacity>
                            ))}

                            {/* Category Filter */}
                            <Text style={styles.filterSectionTitle}>Genre</Text>
                            <TouchableOpacity
                                style={[
                                    styles.filterOption,
                                    !selectedCategory && styles.filterOptionSelected
                                ]}
                                onPress={() => setSelectedCategory(null)}
                            >
                                <Text style={styles.filterOptionText}>All Genres</Text>
                                {!selectedCategory && <Text style={styles.modalCheck}>✓</Text>}
                            </TouchableOpacity>

                            {allCategories.map((category) => (
                                <TouchableOpacity
                                    key={category}
                                    style={[
                                        styles.filterOption,
                                        selectedCategory === category && styles.filterOptionSelected
                                    ]}
                                    onPress={() => setSelectedCategory(category)}
                                >
                                    <Text style={styles.filterOptionText}>{category}</Text>
                                    {selectedCategory === category && <Text style={styles.modalCheck}>✓</Text>}
                                </TouchableOpacity>
                            ))}
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
    toggleControlsButton: {
        padding: theme.spacing.xs,
        paddingHorizontal: theme.spacing.sm,
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
    separator: {
        height: theme.spacing.sm,
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
});
