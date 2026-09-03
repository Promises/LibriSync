/**
 * Provider-agnostic account sync.
 *
 * Every account-management screen syncs through here rather than calling a
 * provider's bridge function directly. Adding a provider means one arm in
 * {@link syncProviderAccount} — the per-account loop, the aggregation and the
 * "sync all" reporting are already shared.
 */
import * as SecureStore from 'expo-secure-store';
import {
  accountProvider,
  providerSyncLibrary,
  refreshToken,
  saveAccount,
  syncLibrary,
} from '../../modules/expo-rust-bridge';
import type { Account, ProviderId, SyncStats } from '../../modules/expo-rust-bridge';

/** Per-page progress, so a screen can show counts climbing during a sync. */
export type SyncProgress = (stats: SyncStats, page: number, aggregated: SyncStats) => void;

export interface SyncAllResult {
  /** Accounts that synced without throwing. */
  succeeded: number;
  /** Display names of accounts that failed, for the summary alert. */
  failed: string[];
  /** Full stats across every account, so the sync report covers a multi-account run. */
  totals: SyncStats;
}

/**
 * Mint a new access token for an account and persist it, unconditionally.
 *
 * Exported so the Accounts screen's "Refresh Token" button and the automatic
 * pre-sync refresh below mint tokens exactly the same way.
 */
export async function refreshAccountToken(dbPath: string, account: Account): Promise<Account> {
  const newTokens = await refreshToken(account);
  const newExpiry = new Date(Date.now() + parseInt(newTokens.expires_in.toString(), 10) * 1000);
  const refreshed: Account = {
    ...account,
    identity: {
      ...account.identity!,
      access_token: { token: newTokens.access_token, expires_at: newExpiry.toISOString() },
      refresh_token: newTokens.refresh_token || account.identity!.refresh_token,
    },
  };

  await saveAccount(dbPath, refreshed);
  return refreshed;
}

/**
 * Audible access tokens are short-lived, so refresh before a sync when the token
 * is nearly expired. Returns the account to sync with — refreshed and persisted,
 * or the original when no refresh was needed.
 *
 * Only Audible has refreshable tokens today; Libro.fm's bearer token doesn't expire
 * on a schedule we can see, and its provider's `refresh` is a no-op.
 */
async function withFreshToken(dbPath: string, account: Account): Promise<Account> {
  if (accountProvider(account) !== 'audible') return account;

  const expiresAt = account.identity?.access_token?.expires_at;
  if (!expiresAt) return account;

  const minutesUntilExpiry = (new Date(expiresAt).getTime() - Date.now()) / 1000 / 60;
  if (minutesUntilExpiry >= 5) return account;

  return refreshAccountToken(dbPath, account);
}

/**
 * Sync one account's owned library, dispatching on its provider.
 *
 * Returns both the stats and the (possibly token-refreshed) account, so callers
 * that hold the account in UI state can keep it current.
 */
export async function syncProviderAccount(
  dbPath: string,
  account: Account,
  onProgress?: SyncProgress,
): Promise<{ stats: SyncStats; account: Account }> {
  const provider: ProviderId = accountProvider(account);
  const synced = await withFreshToken(dbPath, account);

  const stats =
    provider === 'audible'
      ? await syncLibrary(dbPath, synced, onProgress)
      : await providerSyncLibrary(provider, dbPath, synced, onProgress);

  return { stats, account: synced };
}

/**
 * Sync several accounts in sequence, collecting per-account failures rather than
 * aborting the run. Sequential on purpose: concurrent syncs would interleave
 * writes to the same SQLite file and make progress reporting meaningless.
 */
export async function syncAllProviderAccounts(
  dbPath: string,
  accounts: Account[],
  formatName: (account: Account) => string,
  onProgress?: SyncProgress,
): Promise<SyncAllResult> {
  const totals: SyncStats = {
    total_items: 0,
    total_library_count: 0,
    books_added: 0,
    books_updated: 0,
    books_absent: 0,
    errors: [],
    items_failed: 0,
    has_more: false,
    pages: [],
  };
  const failed: string[] = [];

  for (const account of accounts) {
    const name = formatName(account);
    try {
      const { stats } = await syncProviderAccount(dbPath, account, onProgress);
      totals.total_items += stats.total_items;
      totals.total_library_count += stats.total_library_count;
      totals.books_added += stats.books_added;
      totals.books_updated += stats.books_updated;
      totals.books_absent += stats.books_absent;
      totals.items_failed += stats.items_failed ?? 0;
      // Tag each line with its account: a mixed report is unreadable otherwise.
      totals.errors.push(...stats.errors.map((error) => `${name}: ${error}`));
      totals.pages!.push(...(stats.pages ?? []));
    } catch (error) {
      console.error(`[providerAccounts] Sync failed for ${account.account_id}:`, error);
      failed.push(name);
      totals.errors.push(`${name}: sync failed: ${(error as any)?.message ?? String(error)}`);
    }
  }

  return { succeeded: accounts.length - failed.length, failed, totals };
}

/** Record the moment of the last successful sync, shown as "Last synced …". */
export async function recordSyncTime(): Promise<Date> {
  const now = new Date();
  await SecureStore.setItemAsync('last_sync_date', now.toISOString());
  return now;
}
