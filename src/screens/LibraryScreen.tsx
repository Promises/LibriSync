import React, {useState, useEffect} from 'react';
import {View, Text, FlatList, TouchableOpacity, RefreshControl, Image, Alert, ActivityIndicator} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {useStyles} from '../hooks/useStyles';
import {useTheme} from '../styles/theme';
import type {Theme} from '../hooks/useStyles';
import {getBooks, initializeDatabase, downloadAndDecryptBook, refreshToken} from '../../modules/expo-rust-bridge';
import type {Book, Account} from '../../modules/expo-rust-bridge';
import {Paths} from 'expo-file-system';
import * as SecureStore from 'expo-secure-store';

const DOWNLOAD_PATH_KEY = 'download_path';

export default function LibraryScreen() {
    const styles = useStyles(createStyles);
    const { colors } = useTheme();
    const [audiobooks, setAudiobooks] = useState<Book[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [totalCount, setTotalCount] = useState(0);
    const [hasMore, setHasMore] = useState(true);
    const [downloadingAsins, setDownloadingAsins] = useState<Set<string>>(new Set());

    // Load books from database on mount
    useEffect(() => {
        loadBooks(true);
    }, []);

    const loadBooks = async (reset: boolean = false) => {
        try {
            const cacheUri = Paths.cache.uri;
            const cachePath = cacheUri.replace('file://', '');
            const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

            console.log('[LibraryScreen] Loading books from:', dbPath);

            // Initialize database first
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

            console.log('[LibraryScreen] Fetching books:', { offset, limit });
            const response = getBooks(dbPath, offset, limit);
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

    const formatDuration = (seconds: number): string => {
        const hours = Math.floor(seconds / 3600);
        const minutes = Math.floor((seconds % 3600) / 60);
        return `${hours}h ${minutes}m`;
    };

    const getCoverUrl = (book: Book): string | null => {
        if (!book.cover_url) return null;
        // Replace _SL500_ with _SL150_ for smaller cover images
        return book.cover_url.replace(/_SL\d+_/, '_SL150_');
    };

    const getStatus = (book: Book): { text: string; color: string } => {
        if (downloadingAsins.has(book.audible_product_id)) {
            return {text: '⬇ Downloading...', color: colors.info};
        }
        if (book.file_path) {
            return {text: '✓ Downloaded', color: colors.success};
        }
        return {text: 'Available', color: colors.textSecondary};
    };

    const handleDownload = async (book: Book) => {
        try {
            // Get account from SecureStore
            const accountData = await SecureStore.getItemAsync('audible_account');
            if (!accountData) {
                Alert.alert('Error', 'Please log in first');
                return;
            }

            let account: Account = JSON.parse(accountData);

            // Check if token is expired and refresh if needed
            if (account.identity?.access_token) {
                const expiresAt = new Date(account.identity.access_token.expires_at);
                const now = new Date();
                const minutesUntilExpiry = (expiresAt.getTime() - now.getTime()) / 1000 / 60;

                if (minutesUntilExpiry < 5) {
                    console.log('[LibraryScreen] Token expiring soon, refreshing...');
                    try {
                        const newTokens = await refreshToken(account);
                        // Update account with new tokens
                        account.identity.access_token.token = newTokens.access_token;
                        account.identity.refresh_token = newTokens.refresh_token;
                        const newExpiresAt = new Date(Date.now() + parseInt(newTokens.expires_in) * 1000).toISOString();
                        account.identity.access_token.expires_at = newExpiresAt;

                        // Save updated account
                        await SecureStore.setItemAsync('audible_account', JSON.stringify(account));
                        console.log('[LibraryScreen] Token refreshed successfully');
                    } catch (refreshError) {
                        console.error('[LibraryScreen] Token refresh failed:', refreshError);
                        Alert.alert('Error', 'Please log in again - token refresh failed');
                        return;
                    }
                }
            }

            // Get download directory from settings
            // Kotlin module properly handles both SAF URIs (content://) and file paths
            const downloadDir = await SecureStore.getItemAsync(DOWNLOAD_PATH_KEY);

            if (!downloadDir) {
                Alert.alert(
                    'Download Directory Not Set',
                    'Please go to Settings and choose a download directory first.',
                    [{ text: 'OK' }]
                );
                return;
            }

            console.log('[LibraryScreen] Download directory:', downloadDir);

            // Mark as downloading
            setDownloadingAsins(prev => new Set(prev).add(book.audible_product_id));

            console.log('[LibraryScreen] Starting download:', book.title, book.audible_product_id);

            // Download and decrypt
            const result = await downloadAndDecryptBook(
                account,
                book.audible_product_id,
                downloadDir,
                'High'
            );

            console.log('[LibraryScreen] Download complete:', result);

            // Update book with file path
            setAudiobooks(prev =>
                prev.map(b =>
                    b.audible_product_id === book.audible_product_id
                        ? {...b, file_path: result.outputPath}
                        : b
                )
            );

            Alert.alert(
                'Download Complete',
                `${book.title}\n\nSaved to: ${result.outputPath}\nDuration: ${Math.floor(result.duration / 3600)}h ${Math.floor((result.duration % 3600) / 60)}m`
            );

        } catch (error: any) {
            console.error('[LibraryScreen] Download error:', error);
            Alert.alert('Download Failed', error.message || 'Unknown error');
        } finally {
            // Remove from downloading
            setDownloadingAsins(prev => {
                const next = new Set(prev);
                next.delete(book.audible_product_id);
                return next;
            });
        }
    };

    const renderItem = ({item}: { item: Book }) => {
        const status = getStatus(item);
        const authorText = (item.authors?.length || 0) > 0 ? item.authors.join(', ') : 'Unknown Author';
        const coverUrl = getCoverUrl(item);
        const isDownloading = downloadingAsins.has(item.audible_product_id);
        const isDownloaded = !!item.file_path;

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
                        <View style={styles.metadata}>
                            <Text style={styles.duration}>{formatDuration(item.duration_seconds)}</Text>
                            <Text style={[styles.status, {color: status.color}]}>
                                {status.text}
                            </Text>
                        </View>
                    </View>
                    {!isDownloaded && !isDownloading && (
                        <TouchableOpacity
                            style={styles.downloadButton}
                            onPress={() => handleDownload(item)}
                        >
                            <Text style={styles.downloadButtonText}>⬇</Text>
                        </TouchableOpacity>
                    )}
                    {isDownloading && (
                        <View style={styles.downloadButton}>
                            <ActivityIndicator size="small" color={colors.info} />
                        </View>
                    )}
                </View>
            </TouchableOpacity>
        );
    };

    return (
        <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
            <View style={styles.header}>
                <Text style={styles.headerTitle}>Library</Text>
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
                    <Text style={styles.emptyText}>No audiobooks yet</Text>
                    <Text style={styles.emptySubtext}>
                        Go to Account tab to sign in and sync your Audible library
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
    headerTitle: {
        ...theme.typography.title,
    },
    headerSubtitle: {
        ...theme.typography.caption,
        marginTop: theme.spacing.xs,
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
});
