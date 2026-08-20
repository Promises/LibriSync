import React, { useCallback, useState } from 'react';
import { View, Text, ScrollView, Alert, ActivityIndicator, TouchableOpacity } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useFocusEffect } from '@react-navigation/native';
import { Ionicons } from '@expo/vector-icons';
import * as SecureStore from 'expo-secure-store';
import AccountsCard from '../components/AccountsCard';
import AccountActions from '../components/AccountActions';
import Button from '../components/Button';
import ProviderPickerSheet from '../components/ProviderPickerSheet';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import {
  accountProvider,
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
import {
  ACCOUNT_PROVIDERS,
  LIBRIVOX_BROWSE,
  providerEntry,
  type ProviderEntry,
} from '../services/providers/registry';
import { SELECTED_AUDIBLE_ACCOUNT_KEY } from '../services/providers/audible';
import { DEMO_ACCOUNT, DEMO_BOOKS } from '../services/demo/demoData';
import { isDemoAccount, isDemoAccountId } from '../services/demo/demoMode';
import { useProviders } from '../contexts/ProvidersContext';

/**
 * Which account this screen currently has selected, across every provider.
 *
 * Each provider's own `selectedAccountKey` is still written alongside it — other
 * screens (LibraryScreen, download paths) read those, and they must not start
 * seeing another provider's account id.
 */
const SELECTED_ACCOUNT_KEY = 'selected_account_id';

/** A count-only SyncStats, for showing "N audiobooks" before any sync has run. */
function bookCountStats(total: number): SyncStats {
  return {
    total_items: total,
    total_library_count: total,
    books_added: 0,
    books_updated: 0,
    books_absent: 0,
    errors: [],
    has_more: false,
  };
}

/**
 * One screen for every provider's accounts.
 *
 * Replaces the old Providers list → per-provider account screen hop: accounts
 * from all providers live in a single list, Add Account asks which provider, and
 * LibriVox — which has no account — is a browse action in the header.
 *
 * Everything provider-specific comes from `services/providers/registry`.
 */
export default function AccountsScreen({ navigation }: any) {
  const styles = useStyles(createStyles);
  const { colors } = useTheme();
  const { providers: enabledProviders } = useProviders();

  const [accounts, setAccounts] = useState<Account[]>([]);
  const [selected, setSelected] = useState<Account | null>(null);
  const [syncStats, setSyncStats] = useState<SyncStats | null>(null);
  const [lastSyncDate, setLastSyncDate] = useState<Date | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSyncing, setIsSyncing] = useState(false);
  const [showPicker, setShowPicker] = useState(false);
  const [addingProvider, setAddingProvider] = useState<ProviderEntry | null>(null);

  const entryFor = (account: Account) => providerEntry(accountProvider(account));

  /** Providers offerable in the picker — Audible can be switched off in Settings. */
  const availableProviders = ACCOUNT_PROVIDERS.filter(
    (entry) => entry.id !== 'audible' || enabledProviders.audible,
  );

  const loadLibraryCount = useCallback((dbPath: string, account: Account) => {
    if (isDemoAccount(account)) {
      setSyncStats(bookCountStats(DEMO_BOOKS.length));
      return;
    }

    try {
      const result = getBooksWithFilters(
        dbPath, 0, 1, null, null, null, null, null,
        accountProvider(account), null, null, account.account_id, true,
      );
      setSyncStats(bookCountStats(result.total_count));
    } catch (error) {
      console.warn('[AccountsScreen] Could not read book count:', error);
      setSyncStats(null);
    }
  }, []);

  /** Persist the selection globally and under the account's own provider key. */
  const applySelection = useCallback(
    async (dbPath: string, account: Account | null) => {
      setSelected(account);

      if (!account) {
        setSyncStats(null);
        await SecureStore.deleteItemAsync(SELECTED_ACCOUNT_KEY);
        return;
      }

      await SecureStore.setItemAsync(SELECTED_ACCOUNT_KEY, account.account_id);
      await SecureStore.setItemAsync(entryFor(account).selectedAccountKey, account.account_id);
      loadLibraryCount(dbPath, account);
    },
    [loadLibraryCount],
  );

  const loadAccounts = useCallback(async () => {
    try {
      const dbPath = getDatabasePath();
      initializeDatabase(dbPath);

      // Unscoped on purpose — this screen spans every provider.
      const stored = await getAllAccounts(dbPath);

      // The demo account never lives in SQLite; inject it so it behaves like a
      // real Audible account in the list while demo mode is active.
      const audibleSelectedId = await SecureStore.getItemAsync(SELECTED_AUDIBLE_ACCOUNT_KEY);
      const real = stored.filter((acc) => !isDemoAccountId(acc.account_id));
      const merged = isDemoAccountId(audibleSelectedId) ? [DEMO_ACCOUNT, ...real] : real;
      setAccounts(merged);

      const selectedId = await SecureStore.getItemAsync(SELECTED_ACCOUNT_KEY);
      const next =
        merged.find((acc) => acc.account_id === selectedId)
        || merged.find((acc) => acc.account_id === audibleSelectedId)
        || merged[0]
        || null;

      await applySelection(dbPath, next);

      const lastSync = await SecureStore.getItemAsync('last_sync_date');
      setLastSyncDate(lastSync ? new Date(lastSync) : null);
    } catch (error) {
      console.error('[AccountsScreen] Failed to load accounts:', error);
    } finally {
      setIsLoading(false);
    }
  }, [applySelection]);

  useFocusEffect(
    useCallback(() => {
      loadAccounts();
    }, [loadAccounts]),
  );

  const handleSelect = async (account: Account) => {
    await applySelection(getDatabasePath(), account);
  };

  /** Reflect a token-refreshed account back into the list without a reload. */
  const handleAccountUpdated = (updated: Account) => {
    setAccounts((prev) =>
      prev.map((acc) => (acc.account_id === updated.account_id ? updated : acc)),
    );
    setSelected((prev) => (prev?.account_id === updated.account_id ? updated : prev));
  };

  const handleLoginSuccess = async (newAccount: Account) => {
    const provider = addingProvider;
    setAddingProvider(null);

    try {
      const dbPath = getDatabasePath();
      initializeDatabase(dbPath);
      await provider?.persistAccount?.(dbPath, newAccount);
      await SecureStore.setItemAsync(SELECTED_ACCOUNT_KEY, newAccount.account_id);
      await SecureStore.setItemAsync(
        (provider ?? entryFor(newAccount)).selectedAccountKey,
        newAccount.account_id,
      );
    } catch (error) {
      console.error('[AccountsScreen] Failed to save account:', error);
      Alert.alert('Warning', 'Login succeeded but the account could not be saved.');
    }

    await loadAccounts();
  };

  /**
   * Run each provider's pre-sync work once for the whole batch.
   *
   * Returns null when a provider aborts — Audible does this when the user backs
   * out of choosing a download directory.
   */
  const prepareSync = async (
    targets: Account[],
  ): Promise<Array<(dbPath: string) => Promise<string | null>> | null> => {
    const seen = new Set<string>();
    const finishers: Array<(dbPath: string) => Promise<string | null>> = [];

    for (const account of targets) {
      const entry = entryFor(account);
      if (seen.has(entry.id) || !entry.prepareSync) continue;
      seen.add(entry.id);

      const prep = await entry.prepareSync();
      if (!prep.ok) return null;
      if (prep.finish) finishers.push(prep.finish);
    }

    return finishers;
  };

  const runSync = async (targets: Account[]) => {
    if (targets.length === 0) return;

    // Demo mode: the library is a fixed in-memory set — "sync" just refreshes counts.
    if (targets.length === 1 && isDemoAccount(targets[0])) {
      setSyncStats(bookCountStats(DEMO_BOOKS.length));
      setLastSyncDate(new Date());
      Alert.alert('Demo Library', `${DEMO_BOOKS.length} sample audiobooks ready in your library.`);
      return;
    }

    const real = targets.filter((acc) => !isDemoAccount(acc));
    if (real.length === 0) return;

    const finishers = await prepareSync(real);
    if (!finishers) return;

    try {
      setIsSyncing(true);

      const dbPath = getDatabasePath();
      initializeDatabase(dbPath);

      const onProgress = (pageStats: SyncStats, _page: number, aggregated: SyncStats) => {
        setSyncStats({
          ...pageStats,
          total_items: aggregated.total_items,
          books_added: aggregated.books_added,
          books_updated: aggregated.books_updated,
        });
      };

      let summary: string;

      if (real.length === 1) {
        const { stats, account: syncedAccount } = await syncProviderAccount(
          dbPath, real[0], onProgress,
        );
        setSyncStats(stats);
        // A pre-sync token refresh mints a new expiry; keep the detail card current.
        if (syncedAccount !== real[0]) handleAccountUpdated(syncedAccount);
        summary =
          `Synced: ${stats.total_items} / ${stats.total_library_count}\n`
          + `Added: ${stats.books_added}\nUpdated: ${stats.books_updated}`;
      } else {
        const result = await syncAllProviderAccounts(
          dbPath, real, (acc) => entryFor(acc).formatName(acc), onProgress,
        );
        const failSummary = result.failed.length > 0 ? `\nFailed: ${result.failed.join(', ')}` : '';
        summary =
          `Accounts synced: ${result.succeeded} / ${real.length}\n`
          + `Synced: ${result.totals.total_items} / ${result.totals.total_library_count}\n`
          + `Added: ${result.totals.books_added}\nUpdated: ${result.totals.books_updated}`
          + failSummary;
        if (selected) loadLibraryCount(dbPath, selected);
      }

      const extras = await Promise.all(finishers.map((finish) => finish(dbPath)));
      const extraLines = extras.filter(Boolean).join('\n');

      setLastSyncDate(await recordSyncTime());
      Alert.alert('Sync Complete!', extraLines ? `${summary}\n${extraLines}` : summary);
    } catch (error: any) {
      console.error('[AccountsScreen] Sync failed:', error);
      Alert.alert('Sync Failed', error?.message || error?.rustError || 'Failed to sync library');
    } finally {
      setIsSyncing(false);
    }
  };

  const handleSignOut = () => {
    if (!selected) return;

    const entry = entryFor(selected);
    const { title, message } = entry.signOutPrompt(selected);

    Alert.alert(title, message, [
      { text: 'Cancel', style: 'cancel' },
      {
        text: entry.signOutTitle,
        style: 'destructive',
        onPress: async () => {
          try {
            const dbPath = getDatabasePath();
            initializeDatabase(dbPath);
            await entry.signOut(dbPath, selected);
            await SecureStore.deleteItemAsync(SELECTED_ACCOUNT_KEY);
            await loadAccounts();
          } catch (error: any) {
            console.error('[AccountsScreen] Sign out failed:', error);
            Alert.alert('Sign Out Failed', error?.message || 'Could not remove the account.');
          }
        },
      },
    ]);
  };

  // Adding an account takes over the screen — both login flows render full-screen.
  if (addingProvider) {
    const { Login, name } = addingProvider;
    return (
      <Login
        onLoginSuccess={handleLoginSuccess}
        onCancel={() => setAddingProvider(null)}
        title={accounts.length > 0 ? `Add ${name} Account` : `Log in to ${name}`}
      />
    );
  }

  const Details = selected ? entryFor(selected).Details : undefined;

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <View style={styles.header}>
        <Text style={styles.headerTitle}>Accounts</Text>
        {enabledProviders.librivox && (
          <TouchableOpacity
            style={styles.browseButton}
            onPress={() => navigation.navigate('LibriVoxBrowse')}
            accessibilityLabel="Browse LibriVox"
          >
            <Ionicons name={LIBRIVOX_BROWSE.icon} size={18} color={colors[LIBRIVOX_BROWSE.tint]} />
            <Text style={styles.browseButtonText}>Browse LibriVox</Text>
          </TouchableOpacity>
        )}
      </View>

      {isLoading ? (
        <ActivityIndicator style={{ marginTop: 32 }} />
      ) : (
        <ScrollView contentContainerStyle={styles.content}>
          {accounts.length === 0 ? (
            <View style={styles.empty}>
              <Ionicons name="person-add-outline" size={48} color={colors.textSecondary} />
              <Text style={styles.emptyTitle}>No accounts yet</Text>
              <Text style={styles.emptyText}>
                Connect an audiobook source to sync and download your library.
              </Text>
              <Button
                title="Add Account"
                onPress={() => setShowPicker(true)}
                variant="filled"
                state="primary"
                style={{ marginTop: 16, alignSelf: 'stretch' }}
              />
            </View>
          ) : (
            <>
              <AccountsCard
                label="Accounts"
                accounts={accounts}
                selectedAccountId={selected?.account_id ?? null}
                onSelect={handleSelect}
                formatName={(acc) => entryFor(acc).formatName(acc)}
                formatSubtitle={(acc) => entryFor(acc).formatSubtitle?.(acc) ?? null}
                formatIcon={(acc) => {
                  const entry = entryFor(acc);
                  return { name: entry.icon, color: colors[entry.tint] };
                }}
                disabled={isSyncing}
              />

              {!!selected && (
                <>
                  {!!Details && (
                    <Details account={selected} onAccountUpdated={handleAccountUpdated} />
                  )}

                  <View style={styles.card}>
                    <Text style={styles.label}>Library</Text>
                    <Text style={styles.value}>
                      {syncStats
                        ? `${syncStats.total_items} ${syncStats.total_items === 1 ? 'audiobook' : 'audiobooks'}`
                        : 'Not synced yet'}
                    </Text>
                    {!!syncStats
                      && syncStats.total_items > 0
                      && syncStats.total_items < syncStats.total_library_count && (
                      <Text style={styles.caption}>
                        Synced {Math.round((syncStats.total_items / syncStats.total_library_count) * 100)}%
                      </Text>
                    )}
                    {!!lastSyncDate && (
                      <Text style={styles.caption}>Last sync: {lastSyncDate.toLocaleString()}</Text>
                    )}
                  </View>

                  <AccountActions
                    accountCount={accounts.filter((acc) => !isDemoAccount(acc)).length}
                    isSyncing={isSyncing}
                    hasSynced={!!syncStats}
                    onSync={() => runSync([selected])}
                    onSyncAll={() => runSync(accounts)}
                    onSignOut={handleSignOut}
                    signOutTitle={entryFor(selected).signOutTitle}
                  />
                </>
              )}

              <Button
                title="Add Account"
                onPress={() => setShowPicker(true)}
                variant="outlined"
                state="primary"
                disabled={isSyncing}
                style={{ marginTop: 8 }}
              />
            </>
          )}
        </ScrollView>
      )}

      <ProviderPickerSheet
        visible={showPicker}
        providers={availableProviders}
        onSelect={(provider) => {
          setShowPicker(false);
          setAddingProvider(provider);
        }}
        onClose={() => setShowPicker(false)}
      />
    </SafeAreaView>
  );
}

const createStyles = (theme: Theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  header: {
    flexDirection: 'row' as const,
    justifyContent: 'space-between' as const,
    alignItems: 'center' as const,
    padding: theme.spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  headerTitle: {
    ...theme.typography.title,
  },
  browseButton: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    gap: theme.spacing.xs,
    paddingVertical: theme.spacing.xs,
    paddingHorizontal: theme.spacing.sm,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  browseButtonText: {
    ...theme.typography.caption,
    color: theme.colors.textPrimary,
  },
  content: {
    padding: theme.spacing.lg,
    flexGrow: 1,
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
  empty: {
    flex: 1,
    justifyContent: 'center' as const,
    alignItems: 'center' as const,
    paddingHorizontal: theme.spacing.lg,
    gap: theme.spacing.sm,
  },
  emptyTitle: {
    ...theme.typography.subtitle,
    marginTop: theme.spacing.md,
  },
  emptyText: {
    ...theme.typography.caption,
    textAlign: 'center' as const,
  },
});
