/**
 * Libro.fm account behaviour. Libro.fm is DRM-free with a non-expiring bearer token;
 * see `storeAccount.ts` for why that leaves so little here.
 */
import { storeSignOut } from './storeAccount';

export const SELECTED_LIBROFM_ACCOUNT_KEY = 'selected_librofm_account_id';

export const signOutLibroFm = storeSignOut(SELECTED_LIBROFM_ACCOUNT_KEY);
