/**
 * Audible-specific account behaviour, extracted from the old SimpleAccountScreen
 * so the unified Accounts screen can stay provider-agnostic.
 *
 * Anything here exists because Audible needs it and the other providers don't:
 * a region, an expiring token, background workers, and a download directory that
 * must be picked before a sync can write anything.
 */
import { Alert, Platform } from 'react-native';
import * as SecureStore from 'expo-secure-store';
import { Directory, Paths } from 'expo-file-system';
import {
  cancelAllBackgroundTasks,
  deleteAccount,
  saveAccount,
  scanDownloadDirectory,
  scheduleLibrarySync,
  scheduleTokenRefresh,
  setDownloadDirectory,
} from '../../../modules/expo-rust-bridge';
import type { Account } from '../../../modules/expo-rust-bridge';
import { isDemoAccount } from '../demo/demoMode';

/**
 * Which Audible account the app last had selected.
 *
 * LibraryScreen reads this key directly (for demo-mode detection and its default
 * account filter), so it must keep being written even though the Accounts screen
 * now tracks selection across every provider.
 */
export const SELECTED_AUDIBLE_ACCOUNT_KEY = 'selected_audible_account_id';

const DOWNLOAD_PATH_KEY = 'download_path';

/** Audible accounts carry their region on the account, older ones on the identity. */
function getAccountLocale(account: Account | null) {
  return account?.locale || account?.identity?.locale || null;
}

export function getLocaleCode(account: Account | null): string {
  return getAccountLocale(account)?.country_code || '';
}

export function formatAudibleAccountRegion(account: Account): string {
  const locale = getAccountLocale(account);
  const countryCode = locale?.country_code || '';
  const countrySuffix = countryCode ? ` (${countryCode.toUpperCase()})` : '';

  return `${locale?.name || countryCode.toUpperCase() || 'Unknown Region'}${countrySuffix}`;
}

/**
 * Prefer the name the user saved, but ignore a saved name that is nothing but the
 * bare region — that is what an unnamed account ends up with, and the customer
 * name from the API reads better.
 */
export function formatAudibleAccountName(account: Account): string {
  const countryCode = getLocaleCode(account).toUpperCase();
  const bareRegionName = countryCode ? `(${countryCode})` : '';
  const savedName = account.account_name?.trim();

  if (savedName && savedName !== bareRegionName) {
    return savedName;
  }

  const customerName = account.identity?.customer_info?.name?.trim()
    || account.identity?.customer_info?.given_name?.trim();
  const regionSuffix = countryCode ? ` (${countryCode})` : '';

  return `${customerName || 'Audible Account'}${regionSuffix}`;
}

function promptForDownloadDirectory(): Promise<boolean> {
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
}

/**
 * Resolve the download directory, asking the user to pick one the first time.
 *
 * Returns null when the user backs out, which callers treat as "abort the sync"
 * rather than an error.
 */
export async function ensureDownloadDirectory(): Promise<string | null> {
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
    console.error('[providers/audible] Directory picker error:', error);
    Alert.alert('Download Directory Required', error.message || 'Failed to select directory');
    return null;
  }
}

/**
 * Match already-downloaded files in the user's directory to library rows, so a
 * reinstall doesn't present the whole library as un-downloaded.
 *
 * Android-only (SAF); returns null when there is nothing to scan.
 */
export async function linkExistingDownloads(
  dbPath: string,
  downloadDir: string | null,
): Promise<number | null> {
  if (Platform.OS !== 'android' || !downloadDir) return null;

  try {
    const stats = await scanDownloadDirectory(dbPath, downloadDir);
    if (stats.errors.length > 0) {
      console.warn('[providers/audible] Existing download scan warnings:', stats.errors);
    }
    return stats.books_linked;
  } catch (error) {
    console.warn('[providers/audible] Existing download scan failed:', error);
    return null;
  }
}

/** Start the background sync/token workers the user has configured in Settings. */
export async function scheduleWorkersFromSettings(): Promise<void> {
  try {
    const syncFrequency = await SecureStore.getItemAsync('sync_frequency');
    const syncWifiOnly = await SecureStore.getItemAsync('sync_wifi_only');
    const autoTokenRefresh = await SecureStore.getItemAsync('auto_token_refresh');

    if (syncFrequency && syncFrequency !== 'manual') {
      const hours = parseInt(syncFrequency.replace('h', ''), 10);
      scheduleLibrarySync(hours, syncWifiOnly !== 'false');
      console.log(`[providers/audible] Library sync scheduled: every ${hours} hours`);
    }

    // Backup only — a just-in-time refresh already runs before each API call.
    if (autoTokenRefresh !== 'false') {
      scheduleTokenRefresh(24);
      console.log('[providers/audible] Token refresh scheduled: every 24 hours (backup mode)');
    }
  } catch (error) {
    console.error('[providers/audible] Failed to schedule workers:', error);
  }
}

/**
 * Persist a freshly logged-in Audible account.
 *
 * The demo account is deliberately never written to SQLite — it only ever lives
 * in memory and in the selected-account key.
 */
export async function persistAudibleAccount(dbPath: string, account: Account): Promise<void> {
  if (isDemoAccount(account)) return;

  await saveAccount(dbPath, account);
  const expiresAt = account.identity?.access_token?.expires_at;
  if (expiresAt) {
    await SecureStore.setItemAsync('token_expires_at', expiresAt);
  }
  await scheduleWorkersFromSettings();
}

/**
 * Remove an Audible account and the loose SecureStore state that predates
 * accounts living in SQLite.
 *
 * Note `cancelAllBackgroundTasks()` is not Audible-scoped — it stops every
 * provider's workers. That is pre-existing behaviour, kept as-is here.
 */
export async function signOutAudible(dbPath: string, account: Account): Promise<void> {
  try {
    cancelAllBackgroundTasks();
  } catch (error) {
    console.error('[providers/audible] Failed to cancel background tasks:', error);
  }

  // Delete from SQLite first; otherwise a restart or focus reload restores the login.
  if (account.account_id && !isDemoAccount(account)) {
    await deleteAccount(dbPath, account.account_id);
  }

  await Promise.all([
    // Clearing this also leaves demo mode, which LibraryScreen detects from it.
    SecureStore.deleteItemAsync(SELECTED_AUDIBLE_ACCOUNT_KEY),
    SecureStore.deleteItemAsync('audible_account'),
    SecureStore.deleteItemAsync('token_expires_at'),
    SecureStore.deleteItemAsync('last_sync_date'),
    SecureStore.deleteItemAsync('audible_access_token'),
    SecureStore.deleteItemAsync('audible_refresh_token'),
    SecureStore.deleteItemAsync('audible_token_expires_at'),
    SecureStore.deleteItemAsync('audible_device_serial'),
    SecureStore.deleteItemAsync('audible_locale_code'),
  ]);
}
