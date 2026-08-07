import React, { useState, useCallback } from 'react';
import { View, Text, ScrollView, Alert, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useFocusEffect } from '@react-navigation/native';
import * as SecureStore from 'expo-secure-store';
import { useStyles } from '../hooks/useStyles';
import type { Theme } from '../hooks/useStyles';
import AccountsCard from '../components/AccountsCard';
import AccountActions from '../components/AccountActions';
import LibroFmLoginScreen from './LibroFmLoginScreen';
import {
  deleteAccount,
  getAllAccounts,
  getBooksWithFilters,
  initializeDatabase,
} from '../../modules/expo-rust-bridge';
import type { Account, SyncStats } from '../../modules/expo-rust-bridge';
import { getDatabasePath } from '../utils/appPaths';
import {
  recordSyncTime,
  syncAllProviderAccounts,
  syncProviderAccount,
} from '../services/providerAccounts';

const SELECTED_LIBROFM_ACCOUNT_KEY = 'selected_librofm_account_id';

/**
 * Libro.fm account management, mirroring the Audible screen: list accounts, add
 * another, sync the selected one, sync all (only shown with more than one), and
 * sign out.
 *
 * Everything provider-agnostic lives in `AccountsCard` / `AccountActions` /
 * `providerAccounts`; what's left here is Libro.fm's own detail card.
 */
export default function LibroFmAccountScreen() {
  const styles = useStyles(createStyles);

  const [accounts, setAccounts] = useState<Account[]>([]);
  const [account, setAccount] = useState<Account | null>(null);
  const [syncStats, setSyncStats] = useState<SyncStats | null>(null);
  const [lastSyncDate, setLastSyncDate] = useState<Date | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSyncing, setIsSyncing] = useState(false);
  const [showAddAccount, setShowAddAccount] = useState(false);

  const loadAccounts = useCallback(async () => {
    try {
      const dbPath = getDatabasePath();
      initializeDatabase(dbPath);

      const librofmAccounts = await getAllAccounts(dbPath, 'librofm');
      setAccounts(librofmAccounts);

      const selectedId = await SecureStore.getItemAsync(SELECTED_LIBROFM_ACCOUNT_KEY);
      const selected =
        librofmAccounts.find((acc) => acc.account_id === selectedId) || librofmAccounts[0] || null;
      setAccount(selected);

      if (selected) {
        await SecureStore.setItemAsync(SELECTED_LIBROFM_ACCOUNT_KEY, selected.account_id);
        loadBookCount(dbPath, selected);
      } else {
        setSyncStats(null);
      }
    } catch (error) {
      console.error('[LibroFmAccountScreen] Failed to load accounts:', error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useFocusEffect(
    useCallback(() => {
      loadAccounts();
    }, [loadAccounts]),
  );

  /** How many Libro.fm books this account already has in the library. */
  const loadBookCount = (dbPath: string, acc: Account) => {
    try {
      const result = getBooksWithFilters(
        dbPath, 0, 1, null, null, null, null, null, 'librofm', null, null, acc.account_id, true,
      );
      setSyncStats({
        total_items: result.total_count,
        total_library_count: result.total_count,
        books_added: 0,
        books_updated: 0,
        books_absent: 0,
        errors: [],
        has_more: false,
      });
    } catch (error) {
      console.warn('[LibroFmAccountScreen] Could not read book count:', error);
    }
  };

  const handleSelectAccount = async (selected: Account) => {
    setAccount(selected);
    await SecureStore.setItemAsync(SELECTED_LIBROFM_ACCOUNT_KEY, selected.account_id);
    loadBookCount(getDatabasePath(), selected);
  };

  const handleLoginSuccess = async (newAccount: Account) => {
    await SecureStore.setItemAsync(SELECTED_LIBROFM_ACCOUNT_KEY, newAccount.account_id);
    setShowAddAccount(false);
    await loadAccounts();
    setAccount(newAccount);
  };

  const runSync = async (toSync: Account[]) => {
    if (toSync.length === 0) return;
    setIsSyncing(true);
    try {
      const dbPath = getDatabasePath();
      initializeDatabase(dbPath);

      const onProgress = (_stats: SyncStats, _page: number, aggregated: SyncStats) => {
        setSyncStats(aggregated);
      };

      if (toSync.length === 1) {
        const { stats } = await syncProviderAccount(dbPath, toSync[0], onProgress);
        setSyncStats(stats);
        setLastSyncDate(await recordSyncTime());
        Alert.alert(
          'Sync Complete!',
          `Synced: ${stats.total_items}\nAdded: ${stats.books_added}\nUpdated: ${stats.books_updated}`,
        );
        return;
      }

      const result = await syncAllProviderAccounts(dbPath, toSync, formatName, onProgress);
      setLastSyncDate(await recordSyncTime());
      const failSummary = result.failed.length > 0 ? `\nFailed: ${result.failed.join(', ')}` : '';
      Alert.alert(
        result.failed.length > 0 ? 'Sync Finished With Errors' : 'Sync Complete!',
        `Accounts synced: ${result.succeeded} / ${toSync.length}\n` +
          `Synced: ${result.totals.total_items}\nAdded: ${result.totals.books_added}\n` +
          `Updated: ${result.totals.books_updated}${failSummary}`,
      );
    } catch (error: any) {
      console.error('[LibroFmAccountScreen] Sync failed:', error);
      Alert.alert('Sync Failed', error?.message || 'Failed to sync Libro.fm library');
    } finally {
      setIsSyncing(false);
    }
  };

  const handleSignOut = () => {
    if (!account) return;
    Alert.alert(
      'Sign Out',
      `Remove ${account.account_name} from LibriSync? Downloaded files are kept.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Sign Out',
          style: 'destructive',
          onPress: async () => {
            try {
              const dbPath = getDatabasePath();
              await deleteAccount(dbPath, account.account_id);
              await SecureStore.deleteItemAsync(SELECTED_LIBROFM_ACCOUNT_KEY);
              await loadAccounts();
            } catch (error: any) {
              Alert.alert('Sign Out Failed', error?.message || 'Could not remove the account');
            }
          },
        },
      ],
    );
  };

  const formatName = (acc: Account) => acc.account_name || acc.account_id;

  if (isLoading) {
    return (
      <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
        <ActivityIndicator style={{ marginTop: 32 }} />
      </SafeAreaView>
    );
  }

  // No account yet, or explicitly adding one: show the credential form.
  if (!account || showAddAccount) {
    return (
      <LibroFmLoginScreen
        onLoginSuccess={handleLoginSuccess}
        onCancel={account ? () => setShowAddAccount(false) : undefined}
        title={account ? 'Add Libro.fm Account' : 'Libro.fm'}
      />
    );
  }

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.title}>Libro.fm</Text>

        <AccountsCard
          label="Libro.fm Accounts"
          accounts={accounts}
          selectedAccountId={account.account_id}
          onSelect={handleSelectAccount}
          onAddAccount={() => setShowAddAccount(true)}
          formatName={formatName}
          disabled={isSyncing}
        />

        <View style={styles.card}>
          <Text style={styles.label}>Library</Text>
          <Text style={styles.value}>
            {syncStats
              ? `${syncStats.total_items} ${syncStats.total_items === 1 ? 'audiobook' : 'audiobooks'}`
              : 'Not synced yet'}
          </Text>
          {!!lastSyncDate && (
            <Text style={styles.caption}>Last synced: {lastSyncDate.toLocaleString()}</Text>
          )}
        </View>

        <AccountActions
          accountCount={accounts.length}
          isSyncing={isSyncing}
          hasSynced={!!syncStats}
          onSync={() => runSync([account])}
          onSyncAll={() => runSync(accounts)}
          onSignOut={handleSignOut}
          signOutTitle="Sign Out"
        />
      </ScrollView>
    </SafeAreaView>
  );
}

const createStyles = (theme: Theme) => ({
  container: { flex: 1, backgroundColor: theme.colors.background },
  content: { padding: theme.spacing.lg },
  title: { ...theme.typography.title, marginBottom: theme.spacing.md },
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
  value: { ...theme.typography.body },
  caption: { ...theme.typography.caption, marginTop: theme.spacing.xs },
});
