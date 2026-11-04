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
import { accountProvider } from '../../../modules/expo-rust-bridge';
import type { Account, ProviderId } from '../../../modules/expo-rust-bridge';
import type { ColorScheme } from '../../styles/theme';
import LoginScreen from '../../screens/LoginScreen';
import CredentialLoginScreen from '../../screens/CredentialLoginScreen';
import {
  AudibleIcon,
  ChirpIcon,
  LibriVoxIcon,
  LibroFmIcon,
  type ProviderIcon,
} from '../../components/ProviderIcons';
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
import { SELECTED_CHIRP_ACCOUNT_KEY, signOutChirp } from './chirp';

/** Every login screen shares this shape — see LoginScreen / CredentialLoginScreen. */
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
  /** Drawn at the caller's size, in `tint`. */
  Icon: ProviderIcon;
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

  /**
   * Books download through the Rust provider's `DownloadPlan`
   * (`enqueueProviderDownload`). Audible is the one exception: it still builds its
   * download in Kotlin (`enqueueDownloadNew`), and flips to true when it moves onto
   * `enqueueProviderBook`.
   */
  downloadsThroughPlan: boolean;
}

/**
 * A `Login` for a store that signs in with email + password through its Rust
 * provider. Created once per store at module load, so the component identity is
 * stable across renders.
 */
function credentialLogin(
  provider: ProviderId,
  providerName: string,
  blurb: string,
): React.ComponentType<ProviderLoginProps> {
  function StoreLogin(props: ProviderLoginProps) {
    return (
      <CredentialLoginScreen
        {...props}
        provider={provider}
        providerName={providerName}
        blurb={blurb}
      />
    );
  }
  StoreLogin.displayName = `${providerName}Login`;
  return StoreLogin;
}

const AUDIBLE: ProviderEntry = {
  id: 'audible',
  name: 'Audible',
  description: 'Sign in to sync and download your Audible library.',
  Icon: AudibleIcon,
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
  downloadsThroughPlan: false,
  signOutTitle: 'Log Out',
  signOutPrompt: () => ({
    title: 'Logout',
    message: 'Are you sure you want to log out?',
  }),
};

/** The stores' sign-out confirmation: their files stay, only the account goes. */
function storeSignOutPrompt(account: Account) {
  return {
    title: 'Sign Out',
    message: `Remove ${account.account_name} from LibriSync? Downloaded files are kept.`,
  };
}

const LIBROFM: ProviderEntry = {
  id: 'librofm',
  name: 'Libro.fm',
  description: 'Sign in to sync and download your Libro.fm library. DRM-free.',
  Icon: LibroFmIcon,
  tint: 'accent',
  selectedAccountKey: SELECTED_LIBROFM_ACCOUNT_KEY,
  formatName: (account) => account.account_name || account.account_id,
  // CredentialLoginScreen persists the account itself before calling back.
  Login: credentialLogin(
    'librofm',
    'Libro.fm',
    'Sign in to sync and download your Libro.fm audiobooks. DRM-free.',
  ),
  signOut: signOutLibroFm,
  downloadsThroughPlan: true,
  signOutTitle: 'Sign Out',
  signOutPrompt: storeSignOutPrompt,
};

const CHIRP: ProviderEntry = {
  id: 'chirp',
  name: 'Chirp',
  description: 'Sign in to sync and download your Chirp library. DRM-free.',
  Icon: ChirpIcon,
  tint: 'accent',
  selectedAccountKey: SELECTED_CHIRP_ACCOUNT_KEY,
  // account_id is `chirp:<user id>`; the email is the readable name.
  formatName: (account) => account.account_name || account.account_id,
  Login: credentialLogin(
    'chirp',
    'Chirp',
    'Sign in to sync and download your Chirp audiobooks. DRM-free.',
  ),
  signOut: signOutChirp,
  downloadsThroughPlan: true,
  signOutTitle: 'Sign Out',
  signOutPrompt: storeSignOutPrompt,
};

/** Providers you hold an account with, in the order they appear in the picker. */
export const ACCOUNT_PROVIDERS: ProviderEntry[] = [AUDIBLE, LIBROFM, CHIRP];

/**
 * The provider whose Rust `DownloadPlan` downloads a book from `source`, or null
 * when that source downloads some other way (Audible, LibriVox).
 */
export function planDownloadProvider(source: ProviderId | undefined): ProviderEntry | null {
  const entry = ACCOUNT_PROVIDERS.find((e) => e.id === source);
  return entry?.downloadsThroughPlan ? entry : null;
}


/**
 * LibriVox: a catalogue, not an account. Presented as a browse destination.
 */
export const LIBRIVOX_BROWSE = {
  id: 'librivox' as ProviderId,
  name: 'LibriVox',
  description: 'Free public domain audiobooks. No account needed.',
  Icon: LibriVoxIcon,
  tint: 'success' as keyof ColorScheme,
};

/** Every source a book can come from, with its display name — for library filters. */
export const BOOK_SOURCES: { id: ProviderId; name: string }[] = [
  { id: AUDIBLE.id, name: AUDIBLE.name },
  { id: LIBRIVOX_BROWSE.id, name: LIBRIVOX_BROWSE.name },
  ...ACCOUNT_PROVIDERS.filter((e) => e.id !== AUDIBLE.id).map(({ id, name }) => ({ id, name })),
];

/**
 * The entry for an account's provider.
 *
 * Falls back to Audible for rows that predate the `provider` column, matching
 * `accountProvider()`'s `COALESCE(provider,'audible')` behaviour in the bridge.
 */
export function providerEntry(id: ProviderId): ProviderEntry {
  return ACCOUNT_PROVIDERS.find((entry) => entry.id === id) ?? AUDIBLE;
}

/**
 * The line under an account's name in a mixed-provider list: which service it is,
 * then that provider's own detail when it has one ("Audible · United States (US)").
 * The name alone doesn't say — Libro.fm and Chirp accounts are both just an email.
 */
export function formatAccountSubtitle(account: Account): string {
  const entry = providerEntry(accountProvider(account));
  const detail = entry.formatSubtitle?.(account);
  return detail ? `${entry.name} · ${detail}` : entry.name;
}
