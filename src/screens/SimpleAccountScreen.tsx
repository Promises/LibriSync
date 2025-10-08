import React, { useState, useEffect } from 'react';
import { View, Text, TouchableOpacity, StyleSheet, Alert, ScrollView } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as SecureStore from 'expo-secure-store';
import LoginScreen from './LoginScreen';
import { syncLibrary, initializeDatabase, refreshToken, getBooks, getCustomerInformation } from '../../modules/expo-rust-bridge';
import type { Account } from '../../modules/expo-rust-bridge';
import { Paths } from 'expo-file-system';

export default function SimpleAccountScreen() {
  const [account, setAccount] = useState<Account | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSyncing, setIsSyncing] = useState(false);
  const [syncStats, setSyncStats] = useState<any>(null);
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

  // Test connection when account loads
  useEffect(() => {
    if (account?.identity) {
      testConnection();
    }
  }, [account]);

  const loadAccount = async () => {
    try {
      console.log('[SimpleAccountScreen] Loading account from SecureStore');
      const accountJson = await SecureStore.getItemAsync('audible_account');
      if (accountJson) {
        console.log('[SimpleAccountScreen] Account found in storage');
        const loadedAccount = JSON.parse(accountJson);
        setAccount(loadedAccount);

        // Load token expiry
        await loadTokenInfo();

        // Load previously synced book count from database
        await loadSyncedBooks(loadedAccount);
      } else {
        console.log('[SimpleAccountScreen] No account found in storage');
      }
    } catch (error) {
      console.error('[SimpleAccountScreen] Failed to load account:', error);
    } finally {
      setIsLoading(false);
    }
  };

  const loadSyncedBooks = async (acc: Account) => {
    try {
      const cacheUri = Paths.cache.uri;
      const cachePath = cacheUri.replace('file://', '');
      const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

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
        const mockStats = {
          total_items: response.total_count,
          total_library_count: response.total_count,
          books_added: 0,
          books_updated: 0,
          books_absent: 0,
          errors: [],
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

  const loadTokenInfo = async () => {
    try {
      let expiryStr = await SecureStore.getItemAsync('token_expires_at');

      // If not found, try to extract from account identity
      if (!expiryStr && account?.identity) {
        console.log('[SimpleAccountScreen] Token expiry not in SecureStore, extracting from account');
        // Access token is an object with token and expires_at properties
        const accessToken = account.identity.access_token;
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

  const handleLoginSuccess = (newAccount: Account) => {
    console.log('[SimpleAccountScreen] Login successful, setting account');
    setAccount(newAccount);
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
            await SecureStore.deleteItemAsync('audible_account');
            setAccount(null);
            setSyncStats(null);
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

      // Save updated account and token expiry
      await SecureStore.setItemAsync('audible_account', JSON.stringify(updatedAccount));
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

  const handleSyncLibrary = async () => {
    console.log('========== SYNC LIBRARY BUTTON PRESSED ==========');

    if (!account) return;

    try {
      setIsSyncing(true);

      // Initialize database
      const cacheUri = Paths.cache.uri;
      const cachePath = cacheUri.replace('file://', '');
      const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;
      console.log('[SimpleAccountScreen] Database path:', dbPath);
      initializeDatabase(dbPath);

      // Sync library page-by-page with progress updates
      console.log('[SimpleAccountScreen] Starting page-by-page sync...');
      const stats = await syncLibrary(dbPath, account, (pageStats, page) => {
        console.log(`[SimpleAccountScreen] Page ${page} synced: ${pageStats.total_items} items`);
        // Update UI incrementally after each page
        setSyncStats({
          ...pageStats,
          total_items: pageStats.total_items, // This is cumulative in the aggregated stats
          books_added: pageStats.books_added,
          books_updated: pageStats.books_updated,
        });
      });

      // Update UI with final stats
      setSyncStats(stats);
      const now = new Date();
      setLastSyncDate(now);

      // Save last sync timestamp
      await SecureStore.setItemAsync('last_sync_date', now.toISOString());

      Alert.alert(
        'Sync Complete!',
        `Synced: ${stats.total_items} / ${stats.total_library_count}\nAdded: ${stats.books_added}\nUpdated: ${stats.books_updated}`
      );
    } catch (error: any) {
      console.error('Sync failed:', error);
      Alert.alert(
        'Sync Failed',
        error.message || 'Failed to sync library from Audible'
      );
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
              { backgroundColor: connectionStatus === 'connected' ? '#00ff00' : connectionStatus === 'error' ? '#ff3b30' : '#888888' }
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
            <TouchableOpacity
              style={[styles.button, styles.refreshTokenButton]}
              onPress={handleRefreshToken}
              disabled={isRefreshingToken}
            >
              <Text style={styles.buttonText}>
                {isRefreshingToken ? 'Refreshing...' : 'Refresh Token'}
              </Text>
            </TouchableOpacity>
          </View>
        )}

        {syncStats && (
          <View style={styles.card}>
            <Text style={styles.label}>Library</Text>
            <Text style={styles.value}>
              {syncStats.total_items} / {syncStats.total_library_count} audiobooks
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

        <TouchableOpacity
          style={[styles.button, styles.syncButton]}
          onPress={handleSyncLibrary}
          disabled={isSyncing}
        >
          <Text style={styles.buttonText}>
            {isSyncing ? 'Syncing...' : syncStats ? 'Sync Again' : 'Sync Library'}
          </Text>
        </TouchableOpacity>

        <TouchableOpacity
          style={[styles.button, styles.logoutButton]}
          onPress={handleLogout}
        >
          <Text style={[styles.buttonText, styles.logoutButtonText]}>
            Log Out
          </Text>
        </TouchableOpacity>
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#1a1a1a',
  },
  content: {
    padding: 20,
    flexGrow: 1,
  },
  title: {
    fontSize: 32,
    fontWeight: 'bold',
    color: '#ffffff',
    marginBottom: 24,
  },
  card: {
    backgroundColor: '#2a2a2a',
    padding: 16,
    borderRadius: 8,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#333333',
  },
  label: {
    fontSize: 12,
    color: '#888888',
    marginBottom: 4,
    textTransform: 'uppercase',
  },
  value: {
    fontSize: 16,
    color: '#ffffff',
    fontWeight: '600',
  },
  caption: {
    fontSize: 12,
    color: '#888888',
    marginTop: 4,
  },
  statusRow: {
    flexDirection: 'row',
    alignItems: 'center',
  },
  statusIndicator: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: '#00ff00',
    marginRight: 8,
  },
  button: {
    backgroundColor: '#2a2a2a',
    padding: 16,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: '#333333',
    alignItems: 'center',
    marginTop: 12,
  },
  syncButton: {
    backgroundColor: '#ff9500',
    borderColor: '#ff9500',
  },
  refreshTokenButton: {
    marginTop: 8,
    backgroundColor: '#2a2a2a',
    borderColor: '#00ff00',
  },
  logoutButton: {
    borderColor: '#ff3b30',
  },
  buttonText: {
    fontSize: 16,
    fontWeight: '600',
    color: '#ffffff',
  },
  logoutButtonText: {
    color: '#ff3b30',
  },
});
