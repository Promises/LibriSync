/**
 * Chirp account behaviour. Chirp is DRM-free; its session token lasts about a year
 * and has no refresh grant, so expiry means signing in again (the Rust provider says
 * so in its error). See `storeAccount.ts` for the shared parts.
 */
import { storeSignOut } from './storeAccount';

export const SELECTED_CHIRP_ACCOUNT_KEY = 'selected_chirp_account_id';

export const signOutChirp = storeSignOut(SELECTED_CHIRP_ACCOUNT_KEY);
