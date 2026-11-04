/**
 * Account behaviour shared by the DRM-free stores (Libro.fm, Chirp).
 *
 * Deliberately thin: these stores hand out a long-lived bearer token from a password
 * sign-in, so there is no region, no background token worker and no decrypt step to
 * tear down. Everything else they need is already provider-agnostic in
 * `providerAccounts.ts`.
 */
import * as SecureStore from 'expo-secure-store';
import { deleteAccount } from '../../../modules/expo-rust-bridge';
import type { Account } from '../../../modules/expo-rust-bridge';

/** A sign-out that removes the account and forgets it as the store's selection. */
export function storeSignOut(selectedAccountKey: string) {
  return async (dbPath: string, account: Account): Promise<void> => {
    await deleteAccount(dbPath, account.account_id);
    await SecureStore.deleteItemAsync(selectedAccountKey);
  };
}
