/**
 * The TypeScript side of the multi-provider abstraction.
 *
 * The Rust core already owns provider polymorphism (`providers/mod.rs`); this is
 * its UI counterpart, so screens can render "every provider" without a chain of
 * `if (provider === 'audible')`. Adding a provider means adding one entry here.
 *
 * LibriVox is deliberately not an account provider — it needs no login, so it
 * appears as a browse destination ({@link LIBRIVOX_BROWSE}) rather than a row in
 * the account list.
 */
import React from 'react';
import type { Ionicons } from '@expo/vector-icons';
import type { Account, ProviderId } from '../../../modules/expo-rust-bridge';
import type { ColorScheme } from '../../styles/theme';
import LoginScreen from '../../screens/LoginScreen';
import LibroFmLoginScreen from '../../screens/LibroFmLoginScreen';
import AudibleAccountDetails from '../../components/AudibleAccountDetails';
import {
  SELECTED_AUDIBLE_ACCOUNT_KEY,
  ensureDownloadDirectory,
  formatAudibleAccountName,
  formatAudibleAccountRegion,
  linkExistingDownloads,
  persistAudibleAccount,
  signOutAudible,
} from './audible';
import { SELECTED_LIBROFM_ACCOUNT_KEY, signOutLibroFm } from './librofm';

type IoniconName = React.ComponentProps<typeof Ionicons>['name'];

/** Both login screens already share this shape — see LoginScreen / LibroFmLoginScreen. */
export interface ProviderLoginProps {
  onLoginSuccess: (account: Account) => void;
  onCancel?: () => void;
  title?: string;
}

/**
 * What a sync needs to do around the provider-agnostic middle.
 *
 * `ok: false` aborts the sync silently — Audible uses it when the user backs out
 * of picking a download directory. `finish` runs after a successful sync and may
 * return one extra line for the summary alert.
 */
export interface SyncPreparation {
  ok: boolean;
  finish?: (dbPath: string) => Promise<string | null>;
}

export interface ProviderEntry {
  id: ProviderId;
  name: string;
  description: string;
  icon: IoniconName;
  tint: keyof ColorScheme;

  /**
   * SecureStore key holding this provider's last-selected account.
   *
   * Kept per-provider even though the Accounts screen has one global selection:
   * LibraryScreen reads the Audible key directly, and download code paths look up
   * their own provider's account.
   */
  selectedAccountKey: string;

  formatName: (account: Account) => string;
  formatSubtitle?: (account: Account) => string | null;

  /** Extra cards shown when an account of this provider is selected. */
  Details?: React.ComponentType<{
    account: Account;
    onAccountUpdated: (account: Account) => void;
  }>;

  /** Full-screen add-account flow. */
  Login: React.ComponentType<ProviderLoginProps>;
  /** Some login screens save the account themselves; this fills the gap for the rest. */
  persistAccount?: (dbPath: string, account: Account) => Promise<void>;

  prepareSync?: () => Promise<SyncPreparation>;

  signOut: (dbPath: string, account: Account) => Promise<void>;
  signOutTitle: string;
  signOutPrompt: (account: Account) => { title: string; message: string };
}

const AUDIBLE: ProviderEntry = {
  id: 'audible',
  name: 'Audible',
  description: 'Sign in to sync and download your Audible library.',
  icon: 'headset-outline',
  tint: 'accent',
  selectedAccountKey: SELECTED_AUDIBLE_ACCOUNT_KEY,
  formatName: formatAudibleAccountName,
  formatSubtitle: formatAudibleAccountRegion,
  Details: AudibleAccountDetails,
  Login: LoginScreen,
  persistAccount: persistAudibleAccount,
  prepareSync: async () => {
    const downloadDir = await ensureDownloadDirectory();
    if (!downloadDir) return { ok: false };

    return {
      ok: true,
      finish: async (dbPath: string) => {
        const linked = await linkExistingDownloads(dbPath, downloadDir);
        return linked === null ? null : `Existing downloads linked: ${linked}`;
      },
    };
  },
  signOut: signOutAudible,
  signOutTitle: 'Log Out',
  signOutPrompt: () => ({
    title: 'Logout',
    message: 'Are you sure you want to log out?',
  }),
};

const LIBROFM: ProviderEntry = {
  id: 'librofm',
  name: 'Libro.fm',
  description: 'Sign in to sync and download your Libro.fm library. DRM-free.',
  icon: 'storefront-outline',
  tint: 'accent',
  selectedAccountKey: SELECTED_LIBROFM_ACCOUNT_KEY,
  formatName: (account) => account.account_name || account.account_id,
  // LibroFmLoginScreen persists the account itself before calling back.
  Login: LibroFmLoginScreen,
  signOut: signOutLibroFm,
  signOutTitle: 'Sign Out',
  signOutPrompt: (account) => ({
    title: 'Sign Out',
    message: `Remove ${account.account_name} from LibriSync? Downloaded files are kept.`,
  }),
};

/** Providers you hold an account with, in the order they appear in the picker. */
export const ACCOUNT_PROVIDERS: ProviderEntry[] = [AUDIBLE, LIBROFM];

/**
 * LibriVox: a catalogue, not an account. Presented as a browse destination.
 */
export const LIBRIVOX_BROWSE = {
  id: 'librivox' as ProviderId,
  name: 'LibriVox',
  description: 'Free public domain audiobooks. No account needed.',
  icon: 'book-outline' as IoniconName,
  tint: 'success' as keyof ColorScheme,
};

/**
 * The entry for an account's provider.
 *
 * Falls back to Audible for rows that predate the `provider` column, matching
 * `accountProvider()`'s `COALESCE(provider,'audible')` behaviour in the bridge.
 */
export function providerEntry(id: ProviderId): ProviderEntry {
  return ACCOUNT_PROVIDERS.find((entry) => entry.id === id) ?? AUDIBLE;
}
