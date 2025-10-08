/**
 * Expo Rust Bridge - TypeScript Interface
 *
 * This module provides TypeScript bindings for the native Rust core library.
 * All functions communicate with platform-specific native modules (JNI for Android, FFI for iOS).
 */

import { requireNativeModule } from 'expo-modules-core';

// ============================================================================
// Type Definitions
// ============================================================================

/**
 * Generic response wrapper for all Rust function calls.
 * Follows Result<T, E> pattern from Rust.
 */
export interface RustResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

// ----------------------------------------------------------------------------
// OAuth & Authentication Types
// ----------------------------------------------------------------------------

/**
 * OAuth URL generation response containing authorization URL and PKCE parameters.
 */
export interface OAuthUrlData {
  authorization_url: string;
  pkce_verifier: string;
  state: string;
}

/**
 * OAuth token response from Audible API.
 * @deprecated Use RegistrationResponse instead - this flat structure doesn't match actual API response
 */
export interface TokenResponse {
  access_token: string;
  refresh_token: string;
  expires_in: number;
  token_type: string;
}

/**
 * Bearer token information containing access/refresh tokens.
 */
export interface BearerTokenInfo {
  access_token: string;
  refresh_token: string;
  expires_in: string; // String because API returns "3600" as string
}

/**
 * MAC-DMS token information for device authentication.
 */
export interface MacDmsTokenInfo {
  device_private_key: string;
  adp_token: string;
}

/**
 * Cookie information from Audible authentication.
 */
export interface Cookie {
  Name: string;
  Value: string;
  Path: string;
  Domain: string;
  Expires: string;
  IsSecure: boolean;
  IsHttpOnly: boolean;
}

/**
 * Store authentication cookie.
 */
export interface StoreAuthCookie {
  cookie: string;
}

/**
 * Device information from registration.
 */
export interface DeviceInfo {
  device_serial_number: string;
  device_type: string;
  device_name: string;
}

/**
 * Customer information from Audible.
 */
export interface CustomerInfo {
  account_pool: string;
  user_id: string;
  home_region: string;
  name: string;
  given_name?: string;
}

/**
 * Complete registration response from Audible OAuth flow.
 * This is the actual structure returned by exchange_authorization_code().
 */
export interface RegistrationResponse {
  bearer: BearerTokenInfo;
  mac_dms: MacDmsTokenInfo;
  website_cookies: Cookie[];
  store_authentication_cookie: StoreAuthCookie;
  device_info: DeviceInfo;
  customer_info: CustomerInfo;
}

/**
 * Audible marketplace locale configuration.
 */
export interface Locale {
  country_code: string;
  name: string;
  domain: string;
  with_username: boolean;
}

// ----------------------------------------------------------------------------
// Account & Identity Types
// ----------------------------------------------------------------------------

/**
 * User account with credentials and identity information.
 */
export interface Account {
  account_id: string;
  account_name: string;
  library_scan?: boolean;
  decrypt_key?: string;
  locale: Locale;
  identity?: Identity;
}

/**
 * OAuth access token with expiration
 */
export interface AccessToken {
  token: string;
  expires_at: string; // ISO 8601 timestamp
}

/**
 * OAuth identity with access tokens and device information.
 * Complete structure matching Rust Identity
 */
export interface Identity {
  access_token: AccessToken;
  refresh_token: string;
  device_private_key: string;
  adp_token: string;
  cookies: Record<string, string>;
  device_serial_number: string;
  device_type: string;
  device_name: string;
  amazon_account_id: string;
  store_authentication_cookie: string;
  locale: Locale;
  customer_info: {
    account_pool: string;
    user_id: string;
    home_region: string;
    name: string;
    given_name?: string;
  };
}

// ----------------------------------------------------------------------------
// Book & Library Types
// ----------------------------------------------------------------------------

/**
 * Audiobook metadata from local database.
 */
export interface Book {
  id: number;
  audible_product_id: string;
  title: string;
  subtitle?: string;
  authors: string[];
  narrators: string[];
  series_name?: string;
  series_sequence?: number;
  description?: string;
  publisher?: string;
  release_date?: string;
  purchase_date?: string;
  duration_seconds: number;
  language?: string;
  rating?: number;
  cover_url?: string;
  file_path?: string;
  created_at: string;
  updated_at: string;

  // Additional metadata from API
  pdf_url?: string;
  is_finished?: boolean;
  is_downloadable?: boolean;
  is_ayce?: boolean;  // Audible Plus Catalog
  origin_asin?: string;
  episode_number?: number;
  content_delivery_type?: string;
  is_abridged?: boolean;
  is_spatial?: boolean;  // Dolby Atmos
}

/**
 * Library synchronization statistics.
 */
export interface SyncStats {
  total_items: number;
  total_library_count: number;
  books_added: number;
  books_updated: number;
  books_absent: number;
  errors: string[];
  has_more: boolean;
}

// ----------------------------------------------------------------------------
// Download & Progress Types
// ----------------------------------------------------------------------------

/**
 * Download state enumeration.
 */
export type DownloadState = 'Queued' | 'Downloading' | 'Paused' | 'Completed' | 'Failed' | 'Cancelled';

/**
 * Real-time download progress information.
 */
export interface DownloadProgress {
  asin: string;
  title: string;
  bytes_downloaded: number;
  total_bytes: number;
  percent_complete: number;
  download_speed: number; // bytes/sec
  eta_seconds: number;
  state: DownloadState;
}

// ============================================================================
// Native Module Interface
// ============================================================================

/**
 * Native module interface for Rust bridge.
 *
 * This interface defines all available functions exposed by the native Rust library.
 * Functions are implemented in:
 * - Android: src/jni_bridge.rs -> ExpoRustBridgeModule.kt
 * - iOS: UniFFI-generated bindings (future)
 */
export interface ExpoRustBridgeModule {
  // --------------------------------------------------------------------------
  // Authentication
  // --------------------------------------------------------------------------

  /**
   * Generate an OAuth authorization URL for Audible authentication.
   *
   * @param localeCode - The Audible locale (e.g., 'us', 'uk', 'de')
   * @param deviceSerial - 32-character hex device serial number
   * @returns OAuth URL, PKCE verifier, and state for the authorization flow
   *
   * @example
   * ```typescript
   * const result = ExpoRustBridge.generateOAuthUrl('us', deviceSerial);
   * if (result.success) {
   *   const { url, pkce_verifier } = result.data;
   *   // Open URL in WebView
   * }
   * ```
   */
  generateOAuthUrl(localeCode: string, deviceSerial: string): RustResponse<OAuthUrlData>;

  /**
   * Parse OAuth callback URL to extract authorization code.
   *
   * @param callbackUrl - The callback URL received from OAuth redirect
   * @returns Authorization code extracted from callback
   */
  parseOAuthCallback(callbackUrl: string): RustResponse<{ authorization_code: string }>;

  /**
   * Exchange authorization code for complete registration response.
   *
   * @param localeCode - The Audible locale
   * @param authCode - Authorization code from OAuth callback
   * @param deviceSerial - Device serial used for OAuth request
   * @param pkceVerifier - PKCE verifier from generateOAuthUrl
   * @returns Complete registration data including bearer tokens, device info, and customer info
   */
  exchangeAuthCode(
    localeCode: string,
    authCode: string,
    deviceSerial: string,
    pkceVerifier: string
  ): Promise<RustResponse<RegistrationResponse>>;

  /**
   * Refresh an expired access token using a refresh token.
   *
   * @param localeCode - The Audible locale
   * @param refreshToken - Valid refresh token
   * @param deviceSerial - Device serial number
   * @returns New access token and refresh token
   */
  refreshAccessToken(
    localeCode: string,
    refreshToken: string,
    deviceSerial: string
  ): Promise<RustResponse<TokenResponse>>;

  /**
   * Retrieve activation bytes for DRM decryption.
   *
   * @param localeCode - The Audible locale
   * @param accessToken - Valid access token
   * @returns 8-character hex activation bytes
   */
  getActivationBytes(
    localeCode: string,
    accessToken: string
  ): Promise<RustResponse<{ activation_bytes: string }>>;

  // --------------------------------------------------------------------------
  // Database
  // --------------------------------------------------------------------------

  /**
   * Initialize SQLite database with schema.
   *
   * @param dbPath - Absolute path to database file
   * @returns Initialization success status
   */
  initDatabase(dbPath: string): RustResponse<{ initialized: boolean }>;

  /**
   * Retrieve books from database with pagination.
   *
   * @param dbPath - Absolute path to database file
   * @param offset - Number of records to skip
   * @param limit - Maximum number of records to return
   * @returns Array of books and total count
   */
  getBooks(dbPath: string, offset: number, limit: number): RustResponse<{ books: Book[]; total_count: number }>;

  /**
   * Search books by title, author, or narrator.
   *
   * @param dbPath - Absolute path to database file
   * @param query - Search query string
   * @returns Array of matching books
   */
  searchBooks(dbPath: string, query: string): RustResponse<{ books: Book[] }>;

  /**
   * Synchronize library from Audible API to local database.
   *
   * @param dbPath - Absolute path to database file
   * @param accountJson - JSON-serialized Account object with identity
   * @returns Synchronization statistics
   */
  syncLibrary(dbPath: string, accountJson: string): Promise<RustResponse<SyncStats>>;

  /**
   * Synchronize a single page of library from Audible API.
   *
   * This allows for progressive UI updates by fetching one page at a time.
   * The UI can check `has_more` to determine if there are additional pages.
   *
   * @param dbPath - Absolute path to database file
   * @param accountJson - JSON-serialized Account object with identity
   * @param page - Page number to fetch (1-indexed)
   * @returns Synchronization statistics including has_more flag
   *
   * @example
   * ```typescript
   * let page = 1;
   * let hasMore = true;
   * while (hasMore) {
   *   const result = await ExpoRustBridge.syncLibraryPage(dbPath, accountJson, page);
   *   if (result.success) {
   *     console.log(`Page ${page}: ${result.data.total_items} items`);
   *     hasMore = result.data.has_more;
   *     page++;
   *   } else {
   *     break;
   *   }
   * }
   * ```
   */
  syncLibraryPage(dbPath: string, accountJson: string, page: number): Promise<RustResponse<SyncStats>>;

  // --------------------------------------------------------------------------
  // Download & Decryption
  // --------------------------------------------------------------------------

  /**
   * Download an audiobook from Audible.
   *
   * @param asin - Audible product ID (ASIN)
   * @param licenseJson - JSON-serialized license information
   * @param outputPath - Absolute path for downloaded file
   * @returns Path to downloaded file
   */
  downloadBook(
    asin: string,
    licenseJson: string,
    outputPath: string
  ): Promise<RustResponse<{ file_path: string }>>;

  /**
   * Decrypt AAX audiobook to M4B format.
   *
   * @param inputPath - Path to encrypted AAX file
   * @param outputPath - Path for decrypted M4B file
   * @param activationBytes - 8-character hex activation bytes
   * @returns Path to decrypted file
   */
  decryptAAX(
    inputPath: string,
    outputPath: string,
    activationBytes: string
  ): Promise<RustResponse<{ output_path: string }>>;

  // --------------------------------------------------------------------------
  // Utilities
  // --------------------------------------------------------------------------

  /**
   * Validate activation bytes format.
   *
   * @param activationBytes - Activation bytes to validate
   * @returns Validation result
   */
  validateActivationBytes(activationBytes: string): RustResponse<{ valid: boolean }>;

  /**
   * Get list of supported Audible locales.
   *
   * @returns Array of available locales
   */
  getSupportedLocales(): RustResponse<{ locales: Locale[] }>;

  /**
   * Get customer information from Audible API.
   *
   * @param localeCode - The Audible locale
   * @param accessToken - Valid access token
   * @returns Customer name and email (if available)
   */
  getCustomerInformation(
    localeCode: string,
    accessToken: string
  ): Promise<RustResponse<{ name?: string; given_name?: string; email?: string }>>;

  /**
   * Test bridge functionality and get version information.
   *
   * @returns Bridge status and version
   */
  testBridge(): RustResponse<{ bridgeActive: boolean; rustLoaded: boolean; version: string }>;

  /**
   * Legacy logging function for testing native bridge.
   *
   * @param message - Message to log from Rust
   * @returns Confirmation of logging
   */
  logFromRust(message: string): RustResponse<{ logged: boolean }>;
}

// ============================================================================
// Native Module Import
// ============================================================================

let NativeModule: ExpoRustBridgeModule | null = null;

try {
  NativeModule = requireNativeModule('ExpoRustBridge');
} catch (error) {
  console.error('Failed to load ExpoRustBridge native module:', error);
  throw new Error(
    'ExpoRustBridge native module is not available. ' +
    'Make sure the native code is compiled and linked properly. ' +
    'Run "npm run build:rust:android" or "npm run build:rust:ios" to build native libraries.'
  );
}

if (!NativeModule) {
  throw new Error('ExpoRustBridge native module failed to load');
}

// ============================================================================
// Error Handling
// ============================================================================

/**
 * Custom error class for Rust bridge errors.
 */
class RustBridgeError extends Error {
  constructor(message: string, public readonly rustError?: string) {
    super(message);
    this.name = 'RustBridgeError';
  }
}

/**
 * Unwrap a RustResponse<T> or throw an error.
 *
 * @param response - Response from native module
 * @returns Unwrapped data
 * @throws {RustBridgeError} If response indicates failure
 */
function unwrapResult<T>(response: RustResponse<T>): T {
  if (!response.success || !response.data) {
    throw new RustBridgeError(
      response.error || 'Unknown error from Rust bridge',
      response.error
    );
  }
  return response.data;
}

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Generate a random device serial number.
 *
 * @returns 32-character hex string (16 bytes)
 */
function generateDeviceSerial(): string {
  const bytes = new Uint8Array(16);

  // Use crypto.getRandomValues if available, fallback to Math.random
  if (typeof crypto !== 'undefined' && crypto.getRandomValues) {
    crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = Math.floor(Math.random() * 256);
    }
  }

  return Array.from(bytes)
    .map(b => b.toString(16).padStart(2, '0').toUpperCase())
    .join('');
}

/**
 * OAuth flow helper data returned from initiateOAuth.
 */
interface OAuthFlowData {
  url: string;
  pkceVerifier: string;
  state: string;
  deviceSerial: string;
}

/**
 * Initiate OAuth authentication flow.
 *
 * This function generates the OAuth URL and returns all necessary data
 * for completing the flow.
 *
 * @param localeCode - Audible locale (e.g., 'us', 'uk', 'de')
 * @param deviceSerial - Optional device serial (generates if not provided)
 * @returns OAuth URL and flow parameters
 * @throws {RustBridgeError} If URL generation fails
 *
 * @example
 * ```typescript
 * const flowData = await initiateOAuth('us');
 * // Open flowData.url in WebView
 * // Store flowData.pkceVerifier and flowData.deviceSerial for callback
 * ```
 */
function initiateOAuth(
  localeCode: string,
  deviceSerial?: string
): OAuthFlowData {
  console.log('[ExpoRustBridge] initiateOAuth called with locale:', localeCode);
  const serial = deviceSerial || generateDeviceSerial();
  console.log('[ExpoRustBridge] Using device serial:', serial);

  console.log('[ExpoRustBridge] Calling NativeModule.generateOAuthUrl...');
  const response = NativeModule!.generateOAuthUrl(localeCode, serial);
  console.log('[ExpoRustBridge] Response from native:', response);

  const data = unwrapResult(response);
  console.log('[ExpoRustBridge] OAuth data unwrapped successfully');
  console.log('[ExpoRustBridge] Authorization URL:', data.authorization_url);

  return {
    url: data.authorization_url,
    pkceVerifier: data.pkce_verifier,
    state: data.state,
    deviceSerial: serial,
  };
}

/**
 * Complete OAuth authentication flow after receiving callback.
 *
 * @param callbackUrl - Callback URL from OAuth redirect
 * @param localeCode - Audible locale
 * @param deviceSerial - Device serial from initiateOAuth
 * @param pkceVerifier - PKCE verifier from initiateOAuth
 * @returns Access and refresh tokens
 * @throws {RustBridgeError} If authentication fails
 *
 * @example
 * ```typescript
 * const tokens = await completeOAuthFlow(
 *   callbackUrl,
 *   'us',
 *   flowData.deviceSerial,
 *   flowData.pkceVerifier
 * );
 * // Store tokens.access_token and tokens.refresh_token
 * ```
 */
async function completeOAuthFlow(
  callbackUrl: string,
  localeCode: string,
  deviceSerial: string,
  pkceVerifier: string
): Promise<RegistrationResponse> {
  console.log('[ExpoRustBridge] completeOAuthFlow called');
  console.log('[ExpoRustBridge] Callback URL:', callbackUrl);

  // Parse callback URL
  console.log('[ExpoRustBridge] Parsing callback URL...');
  const parseResponse = NativeModule!.parseOAuthCallback(callbackUrl);
  console.log('[ExpoRustBridge] Parse response:', parseResponse);

  const { authorization_code } = unwrapResult(parseResponse);
  console.log('[ExpoRustBridge] Authorization code extracted:', authorization_code);

  // Exchange authorization code for tokens
  const tokenResponse = await NativeModule!.exchangeAuthCode(
    localeCode,
    authorization_code,
    deviceSerial,
    pkceVerifier
  );

  const tokens = unwrapResult(tokenResponse);

  // Log complete token response for debugging
  console.log('[ExpoRustBridge] ========== COMPLETE TOKEN RESPONSE ==========');
  console.log(JSON.stringify(tokens, null, 2));
  console.log('[ExpoRustBridge] ===============================================');

  return tokens;
}

/**
 * Refresh expired access token.
 *
 * @param account - Account with refresh token
 * @returns New tokens
 * @throws {RustBridgeError} If refresh fails
 *
 * @example
 * ```typescript
 * const newTokens = await refreshToken(account);
 * // Update account.identity with new tokens
 * ```
 */
async function refreshToken(account: Account): Promise<TokenResponse> {
  if (!account.identity?.refresh_token) {
    throw new RustBridgeError('No refresh token available');
  }

  const response = await NativeModule!.refreshAccessToken(
    account.locale.country_code,
    account.identity.refresh_token,
    account.identity.device_serial_number
  );

  return unwrapResult(response);
}

/**
 * Get activation bytes for an account.
 *
 * @param account - Account with access token
 * @returns 8-character hex activation bytes
 * @throws {RustBridgeError} If retrieval fails
 *
 * @example
 * ```typescript
 * const activationBytes = await getActivationBytes(account);
 * // Store in account.decrypt_key
 * ```
 */
async function getActivationBytes(account: Account): Promise<string> {
  if (!account.identity?.access_token) {
    throw new RustBridgeError('No access token available');
  }

  const response = await NativeModule!.getActivationBytes(
    account.locale.country_code,
    account.identity.access_token.token
  );

  const data = unwrapResult(response);
  return data.activation_bytes;
}

/**
 * Initialize database if it doesn't exist.
 *
 * @param dbPath - Absolute path to database file
 * @throws {RustBridgeError} If initialization fails
 */
function initializeDatabase(dbPath: string): void {
  const response = NativeModule!.initDatabase(dbPath);
  unwrapResult(response);
}

/**
 * Sync library from Audible to local database.
 *
 * @param dbPath - Database path
 * @param account - Account with valid access token
 * @returns Sync statistics
 * @throws {RustBridgeError} If sync fails
 */
/**
 * Synchronize library from Audible API, fetching all pages progressively.
 *
 * This function loops through all pages and aggregates the results,
 * allowing the UI to show progress updates for each page.
 *
 * @param dbPath - Path to database file
 * @param account - Account with authentication
 * @param onPageComplete - Optional callback invoked after each page is synced
 * @returns Aggregated sync statistics
 *
 * @example
 * ```typescript
 * const stats = await syncLibrary(dbPath, account, (pageStats, page) => {
 *   console.log(`Page ${page}: ${pageStats.total_items} items synced`);
 *   updateUI(pageStats);
 * });
 * ```
 */
async function syncLibrary(
  dbPath: string,
  account: Account,
  onPageComplete?: (stats: SyncStats, page: number, aggregatedStats: SyncStats) => void
): Promise<SyncStats> {
  const accountJson = JSON.stringify(account);

  // Aggregate stats across all pages
  const aggregatedStats: SyncStats = {
    total_items: 0,
    total_library_count: 0,
    books_added: 0,
    books_updated: 0,
    books_absent: 0,
    errors: [],
    has_more: false,
  };

  let page = 1;
  let hasMore = true;

  while (hasMore) {
    console.log(`[syncLibrary] Fetching page ${page}...`);
    const response = await NativeModule!.syncLibraryPage(dbPath, accountJson, page);
    const pageStats = unwrapResult(response);

    // Aggregate results
    aggregatedStats.total_items += pageStats.total_items;
    aggregatedStats.total_library_count = pageStats.total_library_count; // Use latest count
    aggregatedStats.books_added += pageStats.books_added;
    aggregatedStats.books_updated += pageStats.books_updated;
    aggregatedStats.books_absent += pageStats.books_absent;
    aggregatedStats.errors.push(...pageStats.errors);

    hasMore = pageStats.has_more;

    console.log(
      `[syncLibrary] Page ${page} complete: ${pageStats.total_items} items, ` +
      `${pageStats.books_added} added, ${pageStats.books_updated} updated, ` +
      `has_more=${hasMore}`
    );

    // Notify caller of page completion
    if (onPageComplete) {
      onPageComplete(pageStats, page, aggregatedStats);
    }

    page++;
  }

  console.log(
    `[syncLibrary] All pages synced. Total: ${aggregatedStats.total_items} items, ` +
    `${aggregatedStats.books_added} added, ${aggregatedStats.books_updated} updated`
  );

  return aggregatedStats;
}

/**
 * Get books from database with pagination
 */
function getBooks(dbPath: string, offset: number, limit: number): { books: Book[]; total_count: number } {
  const response = NativeModule!.getBooks(dbPath, offset, limit);
  return unwrapResult(response);
}

/**
 * Get customer information from Audible API
 *
 * @param localeCode - Audible locale
 * @param accessToken - Valid access token
 * @returns Customer name and email
 */
async function getCustomerInformation(
  localeCode: string,
  accessToken: string
): Promise<{ name?: string; given_name?: string; email?: string }> {
  const response = await NativeModule!.getCustomerInformation(localeCode, accessToken);
  return unwrapResult(response);
}

/**
 * Synchronize a single page of library from Audible API.
 *
 * Use this for manual page-by-page control. For automatic pagination,
 * use `syncLibrary()` instead.
 *
 * @param dbPath - Path to database file
 * @param account - Account with authentication
 * @param page - Page number (1-indexed)
 * @returns Sync statistics for this page including has_more flag
 */
async function syncLibraryPage(dbPath: string, account: Account, page: number): Promise<SyncStats> {
  const accountJson = JSON.stringify(account);
  const response = await NativeModule!.syncLibraryPage(dbPath, accountJson, page);
  return unwrapResult(response);
}

// ============================================================================
// Exports
// ============================================================================

export default NativeModule;

export type { OAuthFlowData };

export {
  NativeModule as ExpoRustBridge,
  initiateOAuth,
  completeOAuthFlow,
  refreshToken,
  getActivationBytes,
  initializeDatabase,
  syncLibrary,
  syncLibraryPage,
  getBooks,
  getCustomerInformation,
  generateDeviceSerial,
  unwrapResult,
  RustBridgeError,
};
