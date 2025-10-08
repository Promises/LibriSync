import React, {useState, useEffect} from 'react';
import {View, Text, FlatList, StyleSheet, TouchableOpacity, RefreshControl, Image} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {colors, spacing, typography} from '../styles/theme';
import {getBooks, initializeDatabase} from '../../modules/expo-rust-bridge';
import type {Book} from '../../modules/expo-rust-bridge';
import {Paths} from 'expo-file-system';

export default function LibraryScreen() {
    const [audiobooks, setAudiobooks] = useState<Book[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [totalCount, setTotalCount] = useState(0);
    const [hasMore, setHasMore] = useState(true);

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
        if (book.file_path) {
            return {text: '✓ Downloaded', color: colors.success};
        }
        return {text: 'Available', color: colors.textSecondary};
    };

    const renderItem = ({item}: { item: Book }) => {
        const status = getStatus(item);
        const authorText = (item.authors?.length || 0) > 0 ? item.authors.join(', ') : 'Unknown Author';
        const coverUrl = getCoverUrl(item);

        return (
            <TouchableOpacity style={styles.item}>
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

const styles = StyleSheet.create({
    container: {
        flex: 1,
        backgroundColor: colors.background,
    },
    header: {
        padding: spacing.lg,
        borderBottomWidth: 1,
        borderBottomColor: colors.border,
    },
    headerTitle: {
        ...typography.title,
    },
    headerSubtitle: {
        ...typography.caption,
        marginTop: spacing.xs,
    },
    list: {
        padding: spacing.md,
    },
    item: {
        backgroundColor: colors.backgroundSecondary,
        borderRadius: 8,
        padding: spacing.md,
        borderWidth: 1,
        borderColor: colors.border,
    },
    itemRow: {
        flexDirection: 'row',
        gap: spacing.md,
    },
    cover: {
        width: 80,
        height: 80,
        borderRadius: 4,
        backgroundColor: colors.background,
    },
    coverPlaceholder: {
        width: 80,
        height: 80,
        borderRadius: 4,
        backgroundColor: colors.background,
        justifyContent: 'center',
        alignItems: 'center',
    },
    coverPlaceholderText: {
        fontSize: 32,
    },
    itemContent: {
        flex: 1,
        gap: spacing.xs,
    },
    title: {
        ...typography.subtitle,
        fontSize: 16,
    },
    author: {
        ...typography.caption,
    },
    metadata: {
        flexDirection: 'row',
        justifyContent: 'space-between',
        alignItems: 'center',
        marginTop: spacing.xs,
    },
    duration: {
        ...typography.caption,
        fontFamily: 'monospace',
    },
    status: {
        ...typography.caption,
        fontWeight: '600',
    },
    separator: {
        height: spacing.sm,
    },
    emptyState: {
        flex: 1,
        justifyContent: 'center',
        alignItems: 'center',
        padding: spacing.xl,
    },
    emptyText: {
        ...typography.subtitle,
        marginBottom: spacing.sm,
    },
    emptySubtext: {
        ...typography.caption,
        textAlign: 'center',
    },
    loadingFooter: {
        padding: spacing.md,
        alignItems: 'center',
    },
    loadingText: {
        ...typography.caption,
        color: colors.textSecondary,
    },
});
