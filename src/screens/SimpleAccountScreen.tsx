import React, { useState, useEffect } from 'react';
import { View, Text, Alert, ScrollView, Platform } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useFocusEffect } from '@react-navigation/native';
import * as SecureStore from 'expo-secure-store';
import { Directory, Paths } from 'expo-file-system';
import LoginScreen from './LoginScreen';
import Button from '../components/Button';
import {
  syncLibrary,
  initializeDatabase,
  refreshToken,
  getBooks,
  getCustomerInformation,
  saveAccount,
  getPrimaryAccount,
  deleteAccount,
  scanDownloadDirectory,
  setDownloadDirectory,
  copyTextToClipboard,
  SyncStats,
  cancelAllBackgroundTasks,
  scheduleLibrarySync,
  scheduleTokenRefresh,
} from '../../modules/expo-rust-bridge';
import type { Account } from '../../modules/expo-rust-bridge';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import { getDatabasePath } from '../utils/appPaths';

const DOWNLOAD_PATH_KEY = 'download_path';

export default function SimpleAccountScreen() {
  const styles = useStyles(createStyles);
  const { colors, spacing } = useTheme();
  const [account, setAccount] = useState<Account | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSyncing, setIsSyncing] = useState(false);
  const [syncStats, setSyncStats] = useState<SyncStats | null>(null);
  const [lastSyncDate, setLastSyncDate] = useState<Date | null>(null);
  const [tokenExpiry, setTokenExpiry] = useState<Date | null>(null);
  const [timeRemaining, setTimeRemaining] = useState<number | null>(null);
  const [isRefreshingToken, setIsRefreshingToken] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<'connected' | 'error' | 'checking'>('checking');
  const [accountName, setAccountName] = useState<string | null>(null);

  // Load account on mount
  useEffect(() => {
    console.log('[SimpleAccountScreen] Component mounted, loading account');
    loadAccount();
  }, []);

  // Reload account when tab is focused (e.g., after token refresh from another tab)
  useFocusEffect(
    React.useCallback(() => {
      console.log('[SimpleAccountScreen] Tab focused, reloading account...');
      loadAccount();
    }, [])
  );

  // Test connection when account loads
  useEffect(() => {
    if (account?.identity) {
      testConnection();
    }
  }, [account]);

  const loadAccount = async () => {
    try {
      const dbPath = getDatabasePath();

      console.log('[SimpleAccountScreen] Loading account from SQLite database');
      initializeDatabase(dbPath);

      // Try to load account from SQLite (single source of truth)
      const loadedAccount = await getPrimaryAccount(dbPath);

      if (loadedAccount) {
        console.log('[SimpleAccountScreen] Account found in SQLite database');
        setAccount(loadedAccount);

        // Load token expiry
        await loadTokenInfo(loadedAccount);

        // Load previously synced book count
        await loadSyncedBooks(loadedAccount);
      } else {
        console.log('[SimpleAccountScreen] No account found in SQLite database');
        setAccount(null);
        setSyncStats(null);
        setLastSyncDate(null);
        setTokenExpiry(null);
        setTimeRemaining(null);
        setAccountName(null);
        setConnectionStatus('checking');
      }
    } catch (error) {
      console.error('[SimpleAccountScreen] Failed to load account:', error);
    } finally {
      setIsLoading(false);
    }
  };

  const loadSyncedBooks = async (acc: Account) => {
    try {
      const dbPath = getDatabasePath();

      console.log('[SimpleAccountScreen] Checking for synced books at:', dbPath);

      // Initialize database first
      try {
        initializeDatabase(dbPath);
      } catch (dbError) {
        console.log('[SimpleAccountScreen] Database not initialized yet');
        return;
      }

      // Get first page to see if we have any synced books
      const response = getBooks(dbPath, 0, 1);
      console.log('[SimpleAccountScreen] getBooks response:', response);

      if (response.books && response.books.length > 0) {
        console.log('[SimpleAccountScreen] Found previously synced books!');
        console.log('[SimpleAccountScreen] Total in DB:', response.total_count);

        // Load last sync date
        const lastSyncStr = await SecureStore.getItemAsync('last_sync_date');
        if (lastSyncStr) {
          setLastSyncDate(new Date(lastSyncStr));
          console.log('[SimpleAccountScreen] Last sync:', lastSyncStr);
        }

        // Show that we have synced data with actual counts
        const mockStats: SyncStats = {
          total_items: response.total_count,
          total_library_count: response.total_count,
          books_added: 0,
          books_updated: 0,
          books_absent: 0,
          errors: [],
          has_more: false,
        };
        setSyncStats(mockStats);
        console.log('[SimpleAccountScreen] Library card will show:', response.total_count, 'books');
      } else {
        console.log('[SimpleAccountScreen] No synced books found in database');
      }
    } catch (error) {
      console.log('[SimpleAccountScreen] Error loading sync data:', error);
    }
  };

  const loadTokenInfo = async (sourceAccount: Account | null = account) => {
    try {
      let expiryStr = await SecureStore.getItemAsync('token_expires_at');

      // If not found, try to extract from account identity
      if (!expiryStr && sourceAccount?.identity) {
        console.log('[SimpleAccountScreen] Token expiry not in SecureStore, extracting from account');
        // Access token is an object with token and expires_at properties
        const accessToken = sourceAccount.identity.access_token;
        if (typeof accessToken === 'object' && accessToken.expires_at) {
          expiryStr = accessToken.expires_at;
        }
        // Save it for next time
        if (expiryStr) {
          await SecureStore.setItemAsync('token_expires_at', expiryStr);
        }
      }

      if (expiryStr) {
        const expiry = new Date(expiryStr);
        setTokenExpiry(expiry);
        updateTimeRemaining(expiry);
        console.log('[SimpleAccountScreen] Token expires at:', expiry.toLocaleString());
      } else {
        console.log('[SimpleAccountScreen] No token expiry found');
      }
    } catch (error) {
      console.error('[SimpleAccountScreen] Failed to load token info:', error);
    }
  };

  const updateTimeRemaining = (expiry: Date) => {
    const now = new Date();
    const secondsRemaining = Math.floor((expiry.getTime() - now.getTime()) / 1000);
    setTimeRemaining(Math.max(0, secondsRemaining));
  };

  const formatTimeRemaining = (seconds: number): string => {
    if (seconds < 60) return `${seconds}s`;
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    if (hours > 0) return `${hours}h ${minutes}m`;
    return `${minutes}m`;
  };

  const getErrorMessage = (error: any, fallback: string): string => {
    return error?.message || error?.rustError || fallback;
  };

  const getCopyableErrorText = (title: string, error: any, fallback: string): string => {
    const message = getErrorMessage(error, fallback);
    const details = [
      title,
      '',
      `Message: ${message}`,
      error?.rustError && error.rustError !== message ? `Rust error: ${error.rustError}` : null,
      error?.stack ? `Stack:\n${error.stack}` : null,
    ].filter(Boolean);

    return details.join('\n\n');
  };

  const showCopyableErrorAlert = (title: string, error: any, fallback: string) => {
    const message = getErrorMessage(error, fallback);
    const copyText = getCopyableErrorText(title, error, fallback);

    Alert.alert(
      title,
      message,
      [
        {
          text: 'Copy Error',
          onPress: () => {
            try {
              copyTextToClipboard(copyText);
              Alert.alert('Copied', 'Error text copied to clipboard.');
            } catch (copyError: any) {
              Alert.alert('Copy Failed', copyError.message || 'Failed to copy error text.');
            }
          },
        },
        { text: 'OK', style: 'cancel' },
      ]
    );
  };

  const testConnection = async () => {
    if (!account?.identity) return;

    try {
      setConnectionStatus('checking');
      console.log('[SimpleAccountScreen] Fetching customer information from Audible API...');

      // Get access token
      const accessToken = typeof account.identity.access_token === 'string'
        ? account.identity.access_token
        : account.identity.access_token.token;

      // Fetch fresh customer info from API
      const customerInfo = await getCustomerInformation(
        account.locale.country_code,
        accessToken
      );

      // Update state with fresh data
      setAccountName(customerInfo.name || account.identity.customer_info?.name || null);
      setConnectionStatus('connected');
      console.log('[SimpleAccountScreen] Customer info:', customerInfo);
    } catch (error: any) {
      console.error('[SimpleAccountScreen] Failed to fetch customer info:', error);
      // Fallback to stored name if API call fails
      setAccountName(account.identity.customer_info?.name || null);
      setConnectionStatus('error');
    }
  };

  // Update time remaining every minute
  useEffect(() => {
    if (!tokenExpiry) return;

    const interval = setInterval(() => {
      updateTimeRemaining(tokenExpiry);
    }, 60000);

    return () => clearInterval(interval);
  }, [tokenExpiry]);

  const handleLoginSuccess = async (newAccount: Account) => {
    console.log('[SimpleAccountScreen] Login successful, saving to SQLite');

    try {
      const dbPath = getDatabasePath();

      // Initialize database
      initializeDatabase(dbPath);

      // Save account to SQLite (single source of truth)
      await saveAccount(dbPath, newAccount);
      console.log('[SimpleAccountScreen] Account saved to SQLite database');

      const expiresAt = newAccount.identity?.access_token?.expires_at;
      if (expiresAt) {
        await SecureStore.setItemAsync('token_expires_at', expiresAt);
        const expiry = new Date(expiresAt);
        setTokenExpiry(expiry);
        updateTimeRemaining(expiry);
      }

      // Update state
      setAccount(newAccount);

      // Schedule background workers based on saved settings
      await scheduleWorkersFromSettings();
    } catch (error) {
      console.error('[SimpleAccountScreen] Failed to save account:', error);
      Alert.alert('Warning', 'Login successful but failed to save account data');
    }
  };

  const scheduleWorkersFromSettings = async () => {
    try {
      // Load sync frequency setting
      const syncFrequency = await SecureStore.getItemAsync('sync_frequency');
      const syncWifiOnly = await SecureStore.getItemAsync('sync_wifi_only');
      const autoTokenRefresh = await SecureStore.getItemAsync('auto_token_refresh');

      // Schedule library sync if enabled
      if (syncFrequency && syncFrequency !== 'manual') {
        const hours = parseInt(syncFrequency.replace('h', ''));
        const wifiOnly = syncWifiOnly !== 'false'; // default to true
        scheduleLibrarySync(hours, wifiOnly);
        console.log(`[SimpleAccountScreen] Library sync scheduled: every ${hours} hours`);
      }

      // Schedule token refresh if enabled (default: true)
      // This is a backup - just-in-time refresh happens before each API call
      if (autoTokenRefresh !== 'false') {
        scheduleTokenRefresh(24);
        console.log('[SimpleAccountScreen] Token refresh scheduled: every 24 hours (backup mode)');
      }
    } catch (error) {
      console.error('[SimpleAccountScreen] Failed to schedule workers:', error);
    }
  };

  const handleLogout = () => {
    console.log('========== LOG OUT BUTTON PRESSED ==========');

    Alert.alert(
      'Logout',
      'Are you sure you want to log out?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Logout',
          style: 'destructive',
          onPress: async () => {
            try {
              // Cancel all background workers before logging out
              cancelAllBackgroundTasks();
              console.log('[SimpleAccountScreen] All background tasks cancelled');
            } catch (error) {
              console.error('[SimpleAccountScreen] Failed to cancel background tasks:', error);
            }

            try {
              // Delete from SQLite first; otherwise a restart/focus reload would restore the login.
              if (account?.account_id) {
                const dbPath = getDatabasePath();
                initializeDatabase(dbPath);
                await deleteAccount(dbPath, account.account_id);
                console.log('[SimpleAccountScreen] Account deleted from SQLite database');
              }

              await Promise.all([
                SecureStore.deleteItemAsync('audible_account'),
                SecureStore.deleteItemAsync('token_expires_at'),
                SecureStore.deleteItemAsync('last_sync_date'),
                SecureStore.deleteItemAsync('audible_access_token'),
                SecureStore.deleteItemAsync('audible_refresh_token'),
                SecureStore.deleteItemAsync('audible_token_expires_at'),
                SecureStore.deleteItemAsync('audible_device_serial'),
                SecureStore.deleteItemAsync('audible_locale_code'),
              ]);

              setAccount(null);
              setSyncStats(null);
              setLastSyncDate(null);
              setTokenExpiry(null);
              setTimeRemaining(null);
              setAccountName(null);
              setConnectionStatus('checking');
            } catch (error) {
              console.error('[SimpleAccountScreen] Failed to log out:', error);
              Alert.alert('Logout Failed', 'Could not remove the local account data.');
            }
          },
        },
      ]
    );
  };

  const handleRefreshToken = async () => {
    console.log('========== REFRESH TOKEN BUTTON PRESSED ==========');

    if (!account?.identity) {
      Alert.alert('Error', 'No authentication data available');
      return;
    }

    try {
      setIsRefreshingToken(true);
      console.log('[SimpleAccountScreen] Refreshing access token...');

      // Call Rust bridge to refresh token (pass individual parameters)
      const { ExpoRustBridge } = require('../../modules/expo-rust-bridge');
      const response = await ExpoRustBridge.refreshAccessToken(
        account.locale.country_code,
        account.identity.refresh_token,
        account.identity.device_serial_number
      );

      if (!response.success || !response.data) {
        throw new Error(response.error || 'Failed to refresh token');
      }

      const newTokens = response.data;

      // Calculate new expiry time
      const newExpiry = new Date(Date.now() + parseInt(newTokens.expires_in, 10) * 1000);

      // Update account with new tokens
      const updatedAccount: Account = {
        ...account,
        identity: {
          ...account.identity!,
          access_token: {
            token: newTokens.access_token,
            expires_at: newExpiry.toISOString(),
          },
          refresh_token: newTokens.refresh_token || account.identity!.refresh_token,
        },
      };

      const dbPath = getDatabasePath();

      // Save updated account to SQLite (single source of truth)
      await saveAccount(dbPath, updatedAccount);
      console.log('[SimpleAccountScreen] Updated account saved to SQLite');

      await SecureStore.setItemAsync('token_expires_at', newExpiry.toISOString());

      // Update state
      setAccount(updatedAccount);
      setTokenExpiry(newExpiry);
      updateTimeRemaining(newExpiry);

      Alert.alert('Success', 'Access token refreshed successfully');
      console.log('[SimpleAccountScreen] Token refreshed, new expiry:', newExpiry.toLocaleString());
    } catch (error: any) {
      console.error('[SimpleAccountScreen] Token refresh failed:', error);
      Alert.alert('Error', error.message || 'Failed to refresh token');
    } finally {
      setIsRefreshingToken(false);
    }
  };

  const promptForDownloadDirectory = (): Promise<boolean> => {
    return new Promise(resolve => {
      let settled = false;
      const finish = (value: boolean) => {
        if (!settled) {
          settled = true;
          resolve(value);
        }
      };

      Alert.alert(
        'Download Directory Required',
        'Choose a download directory before syncing your library.',
        [
          { text: 'Cancel', style: 'cancel', onPress: () => finish(false) },
          { text: 'Choose', onPress: () => finish(true) },
        ],
        { cancelable: true, onDismiss: () => finish(false) }
      );
    });
  };

  const ensureDownloadDirectory = async (): Promise<string | null> => {
    try {
      const savedPath = await SecureStore.getItemAsync(DOWNLOAD_PATH_KEY);
      if (savedPath) {
        if (Platform.OS === 'android') {
          setDownloadDirectory(savedPath);
        }
        return savedPath;
      }

      const shouldChoose = await promptForDownloadDirectory();
      if (!shouldChoose) return null;

      const selectedDirectory = await Directory.pickDirectoryAsync(
        Platform.OS === 'android' ? undefined : Paths.document?.uri
      );

      if (!selectedDirectory?.uri) return null;

      await SecureStore.setItemAsync(DOWNLOAD_PATH_KEY, selectedDirectory.uri);
      if (Platform.OS === 'android') {
        setDownloadDirectory(selectedDirectory.uri);
      }

      return selectedDirectory.uri;
    } catch (error: any) {
      console.error('[SimpleAccountScreen] Directory picker error:', error);
      Alert.alert('Download Directory Required', error.message || 'Failed to select directory');
      return null;
    }
  };

  const handleSyncLibrary = async () => {
    console.log('========== SYNC LIBRARY BUTTON PRESSED ==========');

    if (!account) return;

    const downloadDir = await ensureDownloadDirectory();
    if (!downloadDir) return;

    try {
      setIsSyncing(true);

      // Initialize database
      const dbPath = getDatabasePath();
      console.log('[SimpleAccountScreen] Database path:', dbPath);
      initializeDatabase(dbPath);

      // Ensure access token is valid before syncing
      let syncAccount = account;
      if (syncAccount.identity?.access_token) {
        const expiresAt = new Date(syncAccount.identity.access_token.expires_at);
        const minutesUntilExpiry = (expiresAt.getTime() - Date.now()) / 1000 / 60;

        if (minutesUntilExpiry < 5) {
          console.log('[SimpleAccountScreen] Token expiring soon, refreshing before sync...');
          try {
            const newTokens = await refreshToken(syncAccount);
            const newExpiry = new Date(Date.now() + parseInt(newTokens.expires_in.toString(), 10) * 1000);

            syncAccount = {
              ...syncAccount,
              identity: {
                ...syncAccount.identity!,
                access_token: {
                  token: newTokens.access_token,
                  expires_at: newExpiry.toISOString(),
                },
                refresh_token: newTokens.refresh_token || syncAccount.identity!.refresh_token,
              },
            };

            // Persist refreshed account
            await saveAccount(dbPath, syncAccount);
            await SecureStore.setItemAsync('token_expires_at', newExpiry.toISOString());
            setAccount(syncAccount);
            setTokenExpiry(newExpiry);

            console.log('[SimpleAccountScreen] Token refreshed before sync, new expiry:', newExpiry.toLocaleString());
          } catch (refreshError: any) {
            console.error('[SimpleAccountScreen] Token refresh failed before sync:', refreshError);
            Alert.alert('Sync Failed', 'Could not refresh expired token. Please log in again.');
            setIsSyncing(false);
            return;
          }
        }
      }

      // Sync library page-by-page with progress updates
      console.log('[SimpleAccountScreen] Starting page-by-page sync...');
      const stats = await syncLibrary(dbPath, syncAccount, (_pageStats, page, aggregatedStats) => {
        console.log(`[SimpleAccountScreen] Page ${page} synced: ${_pageStats.total_items} items`);
        // Update UI incrementally after each page
        setSyncStats({
          ..._pageStats,
          total_items: aggregatedStats.total_items, // This is cumulative in the aggregated stats
          books_added: aggregatedStats.books_added,
          books_updated: aggregatedStats.books_updated,
        });
      });

      // Update UI with final stats
      setSyncStats(stats);

      let existingDownloadsLinked = 0;
      if (Platform.OS === 'android' && downloadDir) {
        try {
          const downloadScanStats = await scanDownloadDirectory(dbPath, downloadDir);
          existingDownloadsLinked = downloadScanStats.books_linked;
          if (downloadScanStats.errors.length > 0) {
            console.warn('[SimpleAccountScreen] Existing download scan warnings:', downloadScanStats.errors);
          }
        } catch (scanError) {
          console.warn('[SimpleAccountScreen] Existing download scan failed:', scanError);
        }
      }

      const now = new Date();
      setLastSyncDate(now);

      // Save last sync timestamp
      await SecureStore.setItemAsync('last_sync_date', now.toISOString());

      const scanSummary = Platform.OS === 'android' && downloadDir
        ? `\nExisting downloads linked: ${existingDownloadsLinked}`
        : '';

      Alert.alert(
        'Sync Complete!',
        `Synced: ${stats.total_items} / ${stats.total_library_count}\nAdded: ${stats.books_added}\nUpdated: ${stats.books_updated}${scanSummary}`
      );
    } catch (error: any) {
      console.error('Sync failed:', error);
      showCopyableErrorAlert('Sync Failed', error, 'Failed to sync library from Audible');
    } finally {
      setIsSyncing(false);
    }
  };

  // Show login screen if not authenticated
  if (!account) {
    console.log('[SimpleAccountScreen] Rendering LoginScreen (no account)');
    return <LoginScreen onLoginSuccess={handleLoginSuccess} />;
  }

  // Show account info if authenticated
  console.log('[SimpleAccountScreen] Rendering account info (authenticated)');
  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.title}>Account</Text>

        {accountName && (
          <View style={styles.card}>
            <Text style={styles.label}>NAME</Text>
            <Text style={styles.value}>{accountName}</Text>
            {account.identity?.customer_info?.user_id && (
              <Text style={styles.caption}>
                ID: {account.identity.customer_info.user_id.substring(0, 30)}...
              </Text>
            )}
          </View>
        )}

        <View style={styles.card}>
          <Text style={styles.label}>CONNECTION STATUS</Text>
          <View style={styles.statusRow}>
            <View style={[
              styles.statusIndicator,
              { backgroundColor: connectionStatus === 'connected' ? colors.success : connectionStatus === 'error' ? colors.error : colors.textSecondary }
            ]} />
            <Text style={styles.value}>
              {connectionStatus === 'connected' ? 'Connected' : connectionStatus === 'error' ? 'Connection Error' : 'Checking...'}
            </Text>
          </View>
        </View>

        <View style={styles.card}>
          <Text style={styles.label}>Region</Text>
          <Text style={styles.value}>
            {account.locale.name} ({account.locale.country_code.toUpperCase()})
          </Text>
        </View>

        {tokenExpiry && (
          <View style={styles.card}>
            <Text style={styles.label}>Access Token</Text>
            <Text style={styles.value}>
              {tokenExpiry < new Date() ? 'Expired' : 'Active'}
            </Text>
            <Text style={styles.caption}>
              Expires: {tokenExpiry.toLocaleString()}
            </Text>
            {timeRemaining !== null && timeRemaining > 0 && (
              <Text style={styles.caption}>
                Time remaining: {formatTimeRemaining(timeRemaining)}
              </Text>
            )}
            <Button
              title="Refresh Token"
              onPress={handleRefreshToken}
              variant="outlined"
              state="primary"
              loading={isRefreshingToken}
              style={{ marginTop: spacing.sm }}
            />
          </View>
        )}

        {syncStats && (
          <View style={styles.card}>
            <Text style={styles.label}>Library</Text>
            <Text style={styles.value}>
              {syncStats.total_items} audiobooks
            </Text>
            {syncStats.total_items > 0 && syncStats.total_items < syncStats.total_library_count && (
              <Text style={styles.caption}>
                Synced {Math.round((syncStats.total_items / syncStats.total_library_count) * 100)}%
              </Text>
            )}
            {syncStats.total_items === syncStats.total_library_count && syncStats.total_items > 0 && (
              <Text style={styles.caption}>Fully synced</Text>
            )}
            {lastSyncDate && (
              <Text style={styles.caption}>
                Last sync: {lastSyncDate.toLocaleString()}
              </Text>
            )}
          </View>
        )}

        <Button
          title={isSyncing ? 'Syncing...' : syncStats ? 'Sync Again' : 'Sync Library'}
          onPress={handleSyncLibrary}
          variant="filled"
          state="warning"
          disabled={isSyncing}
          style={{ marginTop: spacing.sm }}
        />

        <Button
          title="Log Out"
          onPress={handleLogout}
          variant="outlined"
          state="error"
          style={{ marginTop: spacing.sm }}
        />
      </ScrollView>
    </SafeAreaView>
  );
}

const createStyles = (theme: Theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  content: {
    padding: theme.spacing.lg,
    flexGrow: 1,
  },
  title: {
    ...theme.typography.title,
    marginBottom: theme.spacing.lg,
  },
  card: {
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.md,
    borderRadius: 8,
    marginBottom: theme.spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  label: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.xs,
    textTransform: 'uppercase' as const,
  },
  value: {
    ...theme.typography.body,
    fontWeight: '600' as const,
  },
  caption: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
  },
  statusRow: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
  },
  statusIndicator: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: theme.colors.success,
    marginRight: theme.spacing.sm,
  },
});
