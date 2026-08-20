import React, { useState, useEffect, useRef, useCallback } from 'react';
import {
  View,
  Text,
  FlatList,
  TouchableOpacity,
  TextInput,
  Image,
  Alert,
  ActivityIndicator,
  Modal,
  ScrollView,
} from 'react-native';
import { SafeAreaView, useSafeAreaInsets } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import * as SecureStore from 'expo-secure-store';
import { getDatabasePath } from '../utils/appPaths';
import {
  initializeDatabase,
  insertLibrivoxBook,
  downloadLibrivoxFile,
} from '../../modules/expo-rust-bridge';
import {
  searchBooks,
  getRecentBooks,
  getBookSections,
  getAuthorName,
  getCoverUrl,
  formatDuration,
  type LibriVoxBook,
  type LibriVoxSection,
} from '../services/librivox';

const DOWNLOAD_PATH_KEY = 'download_path';

export default function LibriVoxBrowseScreen({ navigation }: any) {
  const styles = useStyles(createStyles);
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();

  const [books, setBooks] = useState<LibriVoxBook[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [hasSearched, setHasSearched] = useState(false);
  const [offset, setOffset] = useState(0);
  const [hasMore, setHasMore] = useState(true);

  // Detail modal state
  const [selectedBook, setSelectedBook] = useState<LibriVoxBook | null>(null);
  const [sections, setSections] = useState<LibriVoxSection[]>([]);
  const [isLoadingSections, setIsLoadingSections] = useState(false);
  const [isAdding, setIsAdding] = useState(false);
  const [isDownloading, setIsDownloading] = useState(false);
  const [addedBooks, setAddedBooks] = useState<Set<string>>(new Set());

  const searchTimeout = useRef<NodeJS.Timeout | null>(null);
  const PAGE_SIZE = 20;

  // Load recent books on mount
  useEffect(() => {
    loadRecentBooks();
  }, []);

  // Debounced search
  useEffect(() => {
    if (searchTimeout.current) {
      clearTimeout(searchTimeout.current);
    }

    if (!searchQuery.trim()) {
      if (hasSearched) {
        setHasSearched(false);
        loadRecentBooks();
      }
      return;
    }

    searchTimeout.current = setTimeout(() => {
      performSearch(true);
    }, 500);

    return () => {
      if (searchTimeout.current) {
        clearTimeout(searchTimeout.current);
      }
    };
  }, [searchQuery]);

  const loadRecentBooks = async () => {
    setIsLoading(true);
    try {
      const results = await getRecentBooks(0, PAGE_SIZE);
      setBooks(results);
      setOffset(PAGE_SIZE);
      setHasMore(results.length >= PAGE_SIZE);
    } catch (error) {
      console.error('[LibriVoxBrowse] Load recent error:', error);
    } finally {
      setIsLoading(false);
    }
  };

  const performSearch = async (reset: boolean) => {
    if (!searchQuery.trim()) return;

    if (reset) {
      setIsLoading(true);
      setOffset(0);
    } else {
      setIsLoadingMore(true);
    }

    try {
      const currentOffset = reset ? 0 : offset;
      const results = await searchBooks(searchQuery, currentOffset, PAGE_SIZE);

      if (reset) {
        setBooks(results);
      } else {
        setBooks(prev => [...prev, ...results]);
      }

      setOffset(currentOffset + PAGE_SIZE);
      setHasMore(results.length >= PAGE_SIZE);
      setHasSearched(true);
    } catch (error) {
      console.error('[LibriVoxBrowse] Search error:', error);
    } finally {
      setIsLoading(false);
      setIsLoadingMore(false);
    }
  };

  const handleLoadMore = () => {
    if (!isLoadingMore && !isLoading && hasMore) {
      if (searchQuery.trim()) {
        performSearch(false);
      }
    }
  };

  const handleBookPress = async (book: LibriVoxBook) => {
    setSelectedBook(book);
    setSections([]);
    setIsLoadingSections(true);

    try {
      const bookSections = await getBookSections(book.id);
      setSections(bookSections);
    } catch (error) {
      console.error('[LibriVoxBrowse] Load sections error:', error);
    } finally {
      setIsLoadingSections(false);
    }
  };

  const handleAddToLibrary = async (book: LibriVoxBook) => {
    setIsAdding(true);
    try {
      const dbPath = getDatabasePath();
      initializeDatabase(dbPath);

      const authors = (book.authors || []).map(a => getAuthorName(a));

      await insertLibrivoxBook(dbPath, {
        librivox_id: book.id,
        title: book.title,
        authors,
        narrators: [],
        description: book.description || '',
        length_in_minutes: Math.ceil((book.totaltimesecs || 0) / 60),
        language: book.language || 'en',
        cover_url: getCoverUrl(book) || undefined,
      });

      setAddedBooks(prev => new Set(prev).add(book.id));
      Alert.alert('Added', `"${book.title}" has been added to your library.`);
    } catch (error: any) {
      console.error('[LibriVoxBrowse] Add to library error:', error);
      Alert.alert('Error', error.message || 'Failed to add book to library');
    } finally {
      setIsAdding(false);
    }
  };

  const handleDownload = async (book: LibriVoxBook) => {
    const downloadDir = await SecureStore.getItemAsync(DOWNLOAD_PATH_KEY);
    if (!downloadDir) {
      Alert.alert(
        'Download Directory Not Set',
        'Please go to Settings and choose a download directory first.',
      );
      return;
    }

    if (!book.url_zip_file) {
      Alert.alert('Error', 'No download URL available for this book.');
      return;
    }

    // Add to library first if not already added
    if (!addedBooks.has(book.id)) {
      await handleAddToLibrary(book);
    }

    const authorText = (book.authors || []).map(a => getAuthorName(a)).join(', ') || 'Unknown Author';

    setIsDownloading(true);
    try {
      await downloadLibrivoxFile(
        book.id,
        book.title,
        authorText,
        book.url_zip_file,
        downloadDir,
      );

      Alert.alert(
        'Download Started',
        `"${book.title}" is downloading. Check the notification for progress.`,
      );
    } catch (error: any) {
      console.error('[LibriVoxBrowse] Download error:', error);
      Alert.alert('Download Failed', error.message || 'Failed to start download');
    } finally {
      setIsDownloading(false);
    }
  };

  const renderBookItem = useCallback(({ item }: { item: LibriVoxBook }) => {
    const authorText = (item.authors || []).map(a => getAuthorName(a)).join(', ') || 'Unknown Author';
    const coverUrl = getCoverUrl(item);
    const duration = item.totaltimesecs ? formatDuration(item.totaltimesecs) : '';
    const isAdded = addedBooks.has(item.id);

    return (
      <TouchableOpacity
        style={styles.item}
        onPress={() => handleBookPress(item)}
      >
        <View style={styles.itemRow}>
          {coverUrl ? (
            <Image
              source={{ uri: coverUrl }}
              style={styles.cover}
              resizeMode="cover"
            />
          ) : (
            <View style={styles.coverPlaceholder}>
              <Ionicons name="book-outline" size={32} color={colors.textSecondary} />
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
              {duration ? <Text style={styles.duration}>{duration}</Text> : null}
              <Text style={styles.language}>{item.language?.toUpperCase()}</Text>
              {item.num_sections && (
                <Text style={styles.chapters}>{item.num_sections} ch.</Text>
              )}
            </View>
          </View>
          <TouchableOpacity
            style={[styles.addButton, isAdded && styles.addedButton]}
            onPress={() => handleAddToLibrary(item)}
            disabled={isAdded || isAdding}
          >
            <Ionicons
              name={isAdded ? 'checkmark' : 'add'}
              size={20}
              color={isAdded ? colors.success : colors.background}
            />
          </TouchableOpacity>
        </View>
      </TouchableOpacity>
    );
  }, [addedBooks, isAdding, colors, styles]);

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <View style={styles.header}>
        <View style={styles.headerTitleRow}>
          {/* Pushed from the Accounts header, so it needs its own way back. */}
          {navigation.canGoBack() && (
            <TouchableOpacity
              onPress={() => navigation.goBack()}
              style={styles.backButton}
              accessibilityLabel="Back"
            >
              <Ionicons name="chevron-back" size={24} color={colors.textPrimary} />
            </TouchableOpacity>
          )}
          <View style={styles.headerTitleText}>
            <Text style={styles.headerTitle}>Browse LibriVox</Text>
            <Text style={styles.headerSubtitle}>Free public domain audiobooks</Text>
          </View>
        </View>

        <View style={styles.searchContainer}>
          <Ionicons
            name="search"
            size={20}
            color={colors.textSecondary}
            style={styles.searchIcon}
          />
          <TextInput
            style={styles.searchInput}
            placeholder="Search by title..."
            placeholderTextColor={colors.textSecondary}
            value={searchQuery}
            onChangeText={setSearchQuery}
            returnKeyType="search"
          />
          {searchQuery.length > 0 && (
            <TouchableOpacity onPress={() => setSearchQuery('')}>
              <Ionicons name="close-circle" size={20} color={colors.textSecondary} />
            </TouchableOpacity>
          )}
        </View>
      </View>

      {isLoading ? (
        <View style={styles.emptyState}>
          <ActivityIndicator size="large" color={colors.accent} />
          <Text style={styles.emptyText}>Searching LibriVox...</Text>
        </View>
      ) : books.length === 0 ? (
        <View style={styles.emptyState}>
          <Ionicons name="library-outline" size={48} color={colors.textSecondary} />
          <Text style={styles.emptyText}>
            {hasSearched ? 'No results found' : 'Search for audiobooks'}
          </Text>
          <Text style={styles.emptySubtext}>
            {hasSearched
              ? 'Try a different search term'
              : 'Browse over 40,000 free public domain audiobooks'}
          </Text>
        </View>
      ) : (
        <FlatList
          data={books}
          renderItem={renderBookItem}
          keyExtractor={(item) => item.id}
          contentContainerStyle={styles.list}
          ItemSeparatorComponent={() => <View style={styles.separator} />}
          onEndReached={handleLoadMore}
          onEndReachedThreshold={0.5}
          ListFooterComponent={
            isLoadingMore ? (
              <View style={styles.loadingFooter}>
                <ActivityIndicator size="small" color={colors.accent} />
              </View>
            ) : null
          }
        />
      )}

      {/* Book Detail Modal */}
      <Modal
        visible={!!selectedBook}
        transparent
        animationType="slide"
        onRequestClose={() => setSelectedBook(null)}
      >
        <TouchableOpacity
          style={styles.modalOverlay}
          activeOpacity={1}
          onPress={() => setSelectedBook(null)}
        >
          <View style={[styles.modalContent, { paddingBottom: Math.max(insets.bottom, 16) + 16 }]} onStartShouldSetResponder={() => true}>
            {selectedBook && (
              <>
                <View style={styles.modalHeader}>
                  <Text style={styles.modalTitle} numberOfLines={2}>
                    {selectedBook.title}
                  </Text>
                  <TouchableOpacity onPress={() => setSelectedBook(null)}>
                    <Ionicons name="close" size={24} color={colors.textPrimary} />
                  </TouchableOpacity>
                </View>

                <Text style={styles.modalAuthor}>
                  {(selectedBook.authors || []).map(a => getAuthorName(a)).join(', ')}
                </Text>

                <View style={styles.modalMeta}>
                  {selectedBook.totaltimesecs > 0 && (
                    <Text style={styles.modalMetaItem}>
                      {formatDuration(selectedBook.totaltimesecs)}
                    </Text>
                  )}
                  <Text style={styles.modalMetaItem}>
                    {selectedBook.num_sections} chapters
                  </Text>
                  <Text style={styles.modalMetaItem}>
                    {selectedBook.language?.toUpperCase()}
                  </Text>
                </View>

                <ScrollView style={styles.modalScroll}>
                  {selectedBook.description ? (
                    <Text style={styles.modalDescription}>
                      {selectedBook.description.replace(/<[^>]*>/g, '').trim()}
                    </Text>
                  ) : null}

                  {isLoadingSections ? (
                    <ActivityIndicator
                      size="small"
                      color={colors.accent}
                      style={{ marginTop: 16 }}
                    />
                  ) : sections.length > 0 ? (
                    <>
                      <Text style={styles.sectionHeader}>Chapters</Text>
                      {sections.map((section) => (
                        <View key={section.id} style={styles.sectionItem}>
                          <Text style={styles.sectionNumber}>
                            {section.section_number}.
                          </Text>
                          <Text style={styles.sectionTitle} numberOfLines={1}>
                            {section.title}
                          </Text>
                          <Text style={styles.sectionDuration}>
                            {section.playtime}
                          </Text>
                        </View>
                      ))}
                    </>
                  ) : null}
                </ScrollView>

                <View style={styles.modalActions}>
                  <TouchableOpacity
                    style={[
                      styles.modalActionButton,
                      addedBooks.has(selectedBook.id) && styles.modalActionButtonDisabled,
                    ]}
                    onPress={() => handleAddToLibrary(selectedBook)}
                    disabled={addedBooks.has(selectedBook.id) || isAdding}
                  >
                    <Ionicons
                      name={addedBooks.has(selectedBook.id) ? 'checkmark-circle' : 'add-circle'}
                      size={20}
                      color={colors.background}
                    />
                    <Text style={styles.modalActionText}>
                      {addedBooks.has(selectedBook.id) ? 'In Library' : 'Add to Library'}
                    </Text>
                  </TouchableOpacity>

                  <TouchableOpacity
                    style={[styles.modalDownloadButton, isDownloading && styles.modalActionButtonDisabled]}
                    onPress={() => handleDownload(selectedBook)}
                    disabled={isDownloading}
                  >
                    <Ionicons name="download" size={20} color={colors.background} />
                    <Text style={styles.modalActionText}>
                      {isDownloading ? 'Downloading...' : 'Download'}
                    </Text>
                  </TouchableOpacity>
                </View>
              </>
            )}
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
    alignItems: 'center' as const,
  },
  backButton: {
    marginRight: theme.spacing.sm,
    marginLeft: -theme.spacing.sm,
    padding: theme.spacing.xs,
  },
  headerTitleText: {
    flex: 1,
  },
  headerTitle: {
    ...theme.typography.title,
  },
  headerSubtitle: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.md,
  },
  searchContainer: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    backgroundColor: theme.colors.backgroundSecondary,
    borderRadius: 8,
    paddingHorizontal: theme.spacing.md,
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
    alignItems: 'center' as const,
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
    gap: theme.spacing.md,
    alignItems: 'center' as const,
    marginTop: theme.spacing.xs,
  },
  duration: {
    ...theme.typography.caption,
    fontFamily: 'monospace',
  },
  language: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
  },
  chapters: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
  },
  addButton: {
    width: 36,
    height: 36,
    borderRadius: 18,
    backgroundColor: theme.colors.accent,
    justifyContent: 'center' as const,
    alignItems: 'center' as const,
  },
  addedButton: {
    backgroundColor: theme.colors.backgroundTertiary,
  },
  separator: {
    height: theme.spacing.sm,
  },
  emptyState: {
    flex: 1,
    justifyContent: 'center' as const,
    alignItems: 'center' as const,
    padding: theme.spacing.xl,
    gap: theme.spacing.md,
  },
  emptyText: {
    ...theme.typography.subtitle,
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
  // Modal styles
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
    maxHeight: '75%' as unknown as number,
  },
  modalHeader: {
    flexDirection: 'row' as const,
    justifyContent: 'space-between' as const,
    alignItems: 'flex-start' as const,
    marginBottom: theme.spacing.sm,
  },
  modalTitle: {
    ...theme.typography.title,
    fontSize: 20,
    flex: 1,
    marginRight: theme.spacing.md,
  },
  modalAuthor: {
    ...theme.typography.body,
    color: theme.colors.textSecondary,
    marginBottom: theme.spacing.sm,
  },
  modalMeta: {
    flexDirection: 'row' as const,
    gap: theme.spacing.lg,
    marginBottom: theme.spacing.md,
  },
  modalMetaItem: {
    ...theme.typography.caption,
    color: theme.colors.accent,
  },
  modalScroll: {
    maxHeight: 300,
    marginBottom: theme.spacing.md,
  },
  modalDescription: {
    ...theme.typography.body,
    color: theme.colors.textSecondary,
    lineHeight: 22,
  },
  sectionHeader: {
    ...theme.typography.subtitle,
    marginTop: theme.spacing.lg,
    marginBottom: theme.spacing.sm,
    color: theme.colors.accent,
  },
  sectionItem: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    paddingVertical: theme.spacing.sm,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  sectionNumber: {
    ...theme.typography.caption,
    width: 28,
    color: theme.colors.textSecondary,
  },
  sectionTitle: {
    ...theme.typography.body,
    flex: 1,
  },
  sectionDuration: {
    ...theme.typography.caption,
    fontFamily: 'monospace',
    color: theme.colors.textSecondary,
    marginLeft: theme.spacing.sm,
  },
  modalActions: {
    flexDirection: 'row' as const,
    gap: theme.spacing.md,
  },
  modalActionButton: {
    flex: 1,
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    justifyContent: 'center' as const,
    gap: theme.spacing.sm,
    backgroundColor: theme.colors.accent,
    borderRadius: 8,
    paddingVertical: theme.spacing.md,
  },
  modalDownloadButton: {
    flex: 1,
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    justifyContent: 'center' as const,
    gap: theme.spacing.sm,
    backgroundColor: theme.colors.success,
    borderRadius: 8,
    paddingVertical: theme.spacing.md,
  },
  modalActionButtonDisabled: {
    opacity: 0.5,
  },
  modalActionText: {
    ...theme.typography.body,
    fontWeight: '600' as const,
    color: theme.colors.background,
  },
});
