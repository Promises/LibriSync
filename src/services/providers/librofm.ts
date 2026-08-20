/**
 * Libro.fm account behaviour.
 *
 * Deliberately thin: Libro.fm is DRM-free with a non-expiring bearer token, so
 * there is no region, no background token worker and no decrypt step to tear down.
 * Everything else it needs is already provider-agnostic in `providerAccounts.ts`.
 */
import * as SecureStore from 'expo-secure-store';
import { deleteAccount } from '../../../modules/expo-rust-bridge';
import type { Account } from '../../../modules/expo-rust-bridge';

export const SELECTED_LIBROFM_ACCOUNT_KEY = 'selected_librofm_account_id';

export async function signOutLibroFm(dbPath: string, account: Account): Promise<void> {
  await deleteAccount(dbPath, account.account_id);
  await SecureStore.deleteItemAsync(SELECTED_LIBROFM_ACCOUNT_KEY);
}
