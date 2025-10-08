/**
 * Integration Example - Using Expo Rust Bridge in React Native
 *
 * This file demonstrates how to integrate the Rust bridge into
 * a React Native application with proper state management,
 * error handling, and UI feedback.
 */

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  StyleSheet,
  TouchableOpacity,
  FlatList,
  ActivityIndicator,
  Alert,
} from 'react-native';

import {
  initiateOAuth,
  completeOAuthFlow,
  getActivationBytes,
  initializeDatabase,
  syncLibrary,
  unwrapResult,
  RustBridgeError,
} from './index';

import ExpoRustBridge from './index';

import type {
  Account,
  Book,
  Locale,
  SyncStats,
  OAuthFlowData,
} from './index';

// ============================================================================
// Custom Hook: Bridge Test
// ============================================================================

export function useBridgeTest() {
  const [status, setStatus] = useState<{
    bridgeActive: boolean;
    rustLoaded: boolean;
    version: string;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    try {
      const response = ExpoRustBridge!.testBridge();
      const data = unwrapResult(response);
      setStatus(data);
    } catch (err) {
      if (err instanceof RustBridgeError) {
        setError(err.rustError || err.message);
      } else {
        setError('Failed to test bridge');
      }
    }
  }, []);

  return { status, error };
}

// ============================================================================
// Custom Hook: Authentication
// ============================================================================

export function useAuthentication() {
  const [locales, setLocales] = useState<Locale[]>([]);
  const [oauthData, setOauthData] = useState<OAuthFlowData | null>(null);
  const [account, setAccount] = useState<Account | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load available locales on mount
  useEffect(() => {
    try {
      const response = ExpoRustBridge!.getSupportedLocales();
      const { locales: availableLocales } = unwrapResult(response);
      setLocales(availableLocales);
    } catch (err) {
      if (err instanceof RustBridgeError) {
        setError(err.rustError || err.message);
      }
    }
  }, []);

  const startAuth = useCallback((localeCode: string) => {
    try {
      setLoading(true);
      setError(null);

      const flowData = initiateOAuth(localeCode);
      setOauthData(flowData);

      // Return URL to open in WebView
      return flowData.url;
    } catch (err) {
      if (err instanceof RustBridgeError) {
        setError(err.rustError || err.message);
      } else {
        setError('Failed to start authentication');
      }
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  const completeAuth = useCallback(
    async (callbackUrl: string, localeCode: string) => {
      if (!oauthData) {
        setError('No OAuth flow in progress');
        return null;
      }

      try {
        setLoading(true);
        setError(null);

        // Complete OAuth flow
        const tokens = await completeOAuthFlow(
          callbackUrl,
          localeCode,
          oauthData.deviceSerial,
          oauthData.pkceVerifier
        );

        // Find locale
        const locale = locales.find((l) => l.country_code === localeCode);
        if (!locale) {
          throw new Error('Locale not found');
        }

        // Create account
        const newAccount: Account = {
          account_id: `account-${Date.now()}`,
          account_name: `Audible ${locale.country_code.toUpperCase()}`,
          locale,
          identity: {
            access_token: {
              token: tokens.bearer.access_token,
              expires_at: new Date(
                Date.now() + parseInt(tokens.bearer.expires_in) * 1000
              ).toISOString(),
            },
            refresh_token: tokens.bearer.refresh_token,
            device_private_key: tokens.mac_dms.device_private_key,
            adp_token: tokens.mac_dms.adp_token,
            cookies: {},
            device_serial_number: oauthData.deviceSerial,
            device_type: 'A2CZJZGLK2JJVM',
            device_name: 'React Native App',
            amazon_account_id: tokens.customer_info.user_id,
            store_authentication_cookie: tokens.store_authentication_cookie.cookie,
            locale,
            customer_info: tokens.customer_info,
          },
        };

        // Get activation bytes
        const activationBytes = await getActivationBytes(newAccount);
        newAccount.decrypt_key = activationBytes;

        setAccount(newAccount);
        setOauthData(null);

        return newAccount;
      } catch (err) {
        if (err instanceof RustBridgeError) {
          setError(err.rustError || err.message);
        } else {
          setError('Authentication failed');
        }
        return null;
      } finally {
        setLoading(false);
      }
    },
    [oauthData, locales]
  );

  const logout = useCallback(() => {
    setAccount(null);
    setError(null);
  }, []);

  return {
    locales,
    account,
    loading,
    error,
    startAuth,
    completeAuth,
    logout,
  };
}

// ============================================================================
// Custom Hook: Library Management
// ============================================================================

export function useLibrary(dbPath: string, account: Account | null) {
  const [books, setBooks] = useState<Book[]>([]);
  const [syncStats, setSyncStats] = useState<SyncStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Initialize database on mount
  useEffect(() => {
    try {
      initializeDatabase(dbPath);
    } catch (err) {
      if (err instanceof RustBridgeError) {
        setError(err.rustError || err.message);
      }
    }
  }, [dbPath]);

  const sync = useCallback(async () => {
    if (!account) {
      setError('No account available');
      return null;
    }

    try {
      setLoading(true);
      setError(null);

      const stats = await syncLibrary(dbPath, account);
      setSyncStats(stats);

      // Reload books after sync
      const response = ExpoRustBridge!.getBooks(dbPath, 0, 50);
      const { books: syncedBooks } = unwrapResult(response);
      setBooks(syncedBooks);

      return stats;
    } catch (err) {
      if (err instanceof RustBridgeError) {
        setError(err.rustError || err.message);
      } else {
        setError('Sync failed');
      }
      return null;
    } finally {
      setLoading(false);
    }
  }, [dbPath, account]);

  const loadBooks = useCallback(
    (page: number = 0, pageSize: number = 20) => {
      try {
        setLoading(true);
        setError(null);

        const offset = page * pageSize;
        const response = ExpoRustBridge!.getBooks(dbPath, offset, pageSize);
        const { books: loadedBooks } = unwrapResult(response);
        setBooks(loadedBooks);

        return loadedBooks;
      } catch (err) {
        if (err instanceof RustBridgeError) {
          setError(err.rustError || err.message);
        } else {
          setError('Failed to load books');
        }
        return [];
      } finally {
        setLoading(false);
      }
    },
    [dbPath]
  );

  const search = useCallback(
    (query: string) => {
      try {
        setLoading(true);
        setError(null);

        const response = ExpoRustBridge!.searchBooks(dbPath, query);
        const { books: searchResults } = unwrapResult(response);
        setBooks(searchResults);

        return searchResults;
      } catch (err) {
        if (err instanceof RustBridgeError) {
          setError(err.rustError || err.message);
        } else {
          setError('Search failed');
        }
        return [];
      } finally {
        setLoading(false);
      }
    },
    [dbPath]
  );

  return {
    books,
    syncStats,
    loading,
    error,
    sync,
    loadBooks,
    search,
  };
}

// ============================================================================
// Component: Bridge Status
// ============================================================================

export function BridgeStatusComponent() {
  const { status, error } = useBridgeTest();

  if (error) {
    return (
      <View style={styles.container}>
        <Text style={styles.errorText}>Bridge Error: {error}</Text>
      </View>
    );
  }

  if (!status) {
    return (
      <View style={styles.container}>
        <ActivityIndicator size="large" />
      </View>
    );
  }

  return (
    <View style={styles.container}>
      <Text style={styles.title}>Bridge Status</Text>
      <Text style={styles.text}>
        Bridge Active: {status.bridgeActive ? '✅' : '❌'}
      </Text>
      <Text style={styles.text}>
        Rust Loaded: {status.rustLoaded ? '✅' : '❌'}
      </Text>
      <Text style={styles.text}>Version: {status.version}</Text>
    </View>
  );
}

// ============================================================================
// Component: Library Screen
// ============================================================================

export function LibraryScreen() {
  const dbPath = '/path/to/audible.db'; // Use actual path in production
  const account = null; // Get from authentication context

  const { books, loading, error, sync, loadBooks, search } = useLibrary(
    dbPath,
    account
  );

  const handleSync = async () => {
    const stats = await sync();
    if (stats) {
      Alert.alert(
        'Sync Complete',
        `Added: ${stats.books_added}\nUpdated: ${stats.books_updated}`
      );
    }
  };

  const handleSearch = (query: string) => {
    if (query.trim()) {
      search(query);
    } else {
      loadBooks();
    }
  };

  const renderBook = ({ item }: { item: Book }) => (
    <View style={styles.bookItem}>
      <Text style={styles.bookTitle}>{item.title}</Text>
      <Text style={styles.bookAuthor}>{item.authors.join(', ')}</Text>
      <Text style={styles.bookDuration}>
        {Math.round(item.duration_seconds / 3600)} hours
      </Text>
    </View>
  );

  return (
    <View style={styles.container}>
      <TouchableOpacity
        style={styles.button}
        onPress={handleSync}
        disabled={loading || !account}
      >
        <Text style={styles.buttonText}>
          {loading ? 'Syncing...' : 'Sync Library'}
        </Text>
      </TouchableOpacity>

      {error && <Text style={styles.errorText}>{error}</Text>}

      <FlatList
        data={books}
        renderItem={renderBook}
        keyExtractor={(item) => item.id.toString()}
        ListEmptyComponent={
          <Text style={styles.emptyText}>
            {loading ? 'Loading...' : 'No books found'}
          </Text>
        }
      />
    </View>
  );
}

// ============================================================================
// Styles
// ============================================================================

const styles = StyleSheet.create({
  container: {
    flex: 1,
    padding: 16,
    backgroundColor: '#1a1a1a',
  },
  title: {
    fontSize: 24,
    fontWeight: 'bold',
    color: '#ffffff',
    marginBottom: 16,
  },
  text: {
    fontSize: 16,
    color: '#ffffff',
    marginBottom: 8,
  },
  errorText: {
    fontSize: 14,
    color: '#ff4444',
    marginBottom: 8,
  },
  button: {
    backgroundColor: '#4CAF50',
    padding: 16,
    borderRadius: 8,
    marginBottom: 16,
    alignItems: 'center',
  },
  buttonText: {
    color: '#ffffff',
    fontSize: 16,
    fontWeight: 'bold',
  },
  bookItem: {
    backgroundColor: '#2a2a2a',
    padding: 16,
    borderRadius: 8,
    marginBottom: 12,
  },
  bookTitle: {
    fontSize: 18,
    fontWeight: 'bold',
    color: '#ffffff',
    marginBottom: 4,
  },
  bookAuthor: {
    fontSize: 14,
    color: '#888888',
    marginBottom: 4,
  },
  bookDuration: {
    fontSize: 12,
    color: '#666666',
  },
  emptyText: {
    fontSize: 16,
    color: '#666666',
    textAlign: 'center',
    marginTop: 32,
  },
});

// ============================================================================
// Export All
// ============================================================================

export default {
  BridgeStatusComponent,
  LibraryScreen,
  useBridgeTest,
  useAuthentication,
  useLibrary,
};
