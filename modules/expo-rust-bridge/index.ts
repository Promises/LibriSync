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
 * Download task status enumeration.
 */
export type TaskStatus = 'queued' | 'downloading' | 'paused' | 'completed' | 'failed' | 'cancelled';

/**
 * Download task representing a book download.
 */
export interface DownloadTask {
  task_id: string;
  asin: string;
  title: string;
  status: TaskStatus;
  bytes_downloaded: number;
  total_bytes: number;
  download_url: string;
  download_path: string;
  output_path: string;
  request_headers: Record<string, string>;
  error?: string;
  retry_count: number;
  created_at: string;
  started_at?: string;
  completed_at?: string;
}

/**
 * Legacy: Real-time download progress information.
 * @deprecated Use DownloadTask instead
 */
export interface DownloadProgress {
  asin: string;
  title: string;
  bytes_downloaded: number;
  total_bytes: number;
  percent_complete: number;
  download_speed: number; // bytes/sec
  eta_seconds: number;
  state: TaskStatus;
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
   * Get books with advanced filtering, sorting, and search.
   *
   * @param dbPath - Absolute path to database file
   * @param offset - Pagination offset
   * @param limit - Maximum number of records to return
   * @param searchQuery - Optional search query (searches title, author, narrator)
   * @param seriesName - Optional series filter
   * @param category - Optional category/genre filter
   * @param sortField - Sort field: "title" | "release_date" | "date_added"
   * @param sortDirection - Sort direction: "asc" | "desc"
   * @returns Array of books and total count
   */
  getBooksWithFilters(
    dbPath: string,
    offset: number,
    limit: number,
    searchQuery?: string | null,
    seriesName?: string | null,
    category?: string | null,
    sortField?: string | null,
    sortDirection?: string | null
  ): RustResponse<{ books: Book[]; total_count: number }>;

  /**
   * Get all unique series names from the library.
   *
   * @param dbPath - Absolute path to database file
   * @returns Array of series names
   */
  getAllSeries(dbPath: string): RustResponse<{ series: string[] }>;

  /**
   * Get all unique categories/genres from the library.
   *
   * @param dbPath - Absolute path to database file
   * @returns Array of category names
   */
  getAllCategories(dbPath: string): RustResponse<{ categories: string[] }>;

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
   * Download and decrypt an audiobook (complete pipeline).
   *
   * This function handles the entire process:
   * 1. Rust downloads encrypted AAXC file to cache and extracts decryption keys
   * 2. Kotlin decrypts using FFmpeg-Kit (16KB page size compatible)
   * 3. Kotlin copies to user's chosen directory (handles SAF URIs via DocumentFile)
   * 4. Kotlin cleans up cache files
   * 5. Returns final decrypted file path with duration
   *
   * @param accountJson - JSON-serialized Account object with identity
   * @param asin - Audible product ID (ASIN)
   * @param outputDirectory - Directory to save M4B file (supports content:// SAF URIs)
   * @param quality - Download quality ("Low", "Normal", "High", "Extreme")
   * @returns Final file path, size, and duration
   */
  downloadBook(
    accountJson: string,
    asin: string,
    outputDirectory: string,
    quality: string,
    dbPath: string
  ): Promise<RustResponse<{
    outputPath: string;
    fileSize: number;
    duration: number;
  }>>;

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

  // --------------------------------------------------------------------------
  // FFmpeg-Kit Functions (16KB Page Size Compatible)
  // --------------------------------------------------------------------------

  /**
   * Convert AAX/AAXC to M4B using FFmpeg-Kit.
   *
   * @param inputPath - Path to input AAX/AAXC file
   * @param outputPath - Path to output M4B file
   * @param activationBytes - Optional activation bytes for AAX (8 hex chars)
   * @param aaxcKey - Optional AAXC decryption key (hex string)
   * @param aaxcIv - Optional AAXC initialization vector (hex string)
   * @returns Conversion result with output path and file size
   */
  convertToM4b(
    inputPath: string,
    outputPath: string,
    activationBytes?: string,
    aaxcKey?: string,
    aaxcIv?: string
  ): Promise<RustResponse<{ outputPath: string; fileSize: number; returnCode: number }>>;

  /**
   * Convert audio file to different format using FFmpeg-Kit.
   *
   * @param inputPath - Path to input file
   * @param outputPath - Path to output file
   * @param codec - Audio codec (aac, libmp3lame, copy)
   * @param bitrate - Bitrate (e.g., "128k" or null for default)
   * @param quality - VBR quality (0-9 for MP3, null for CBR)
   * @returns Conversion result with output path and file size
   */
  convertAudio(
    inputPath: string,
    outputPath: string,
    codec: string,
    bitrate?: string,
    quality?: number
  ): Promise<RustResponse<{ outputPath: string; fileSize: number }>>;

  /**
   * Get audio file duration and metadata using FFprobe.
   *
   * @param filePath - Path to audio file
   * @returns Duration and metadata information
   */
  getAudioInfo(
    filePath: string
  ): Promise<RustResponse<{ duration: number; bitrate: string; format: string; size: string }>>;

  // --------------------------------------------------------------------------
  // Download Manager
  // --------------------------------------------------------------------------

  /**
   * Enqueue a download using the persistent download manager.
   *
   * @param dbPath - Path to SQLite database
   * @param accountJson - JSON-serialized Account object
   * @param asin - Book ASIN
   * @param title - Book title
   * @param outputDirectory - Output directory (can be SAF URI)
   * @param quality - Download quality
   * @returns Success message
   */
  enqueueDownload(
    dbPath: string,
    accountJson: string,
    asin: string,
    title: string,
    outputDirectory: string,
    quality: string
  ): Promise<RustResponse<{ message: string }>>;

  /**
   * Get download task status.
   *
   * @param dbPath - Path to SQLite database
   * @param taskId - Task ID
   * @returns Task details
   */
  getDownloadTask(dbPath: string, taskId: string): RustResponse<DownloadTask>;

  /**
   * List download tasks with optional filter.
   *
   * @param dbPath - Path to SQLite database
   * @param filter - Optional status filter
   * @returns List of tasks
   */
  listDownloadTasks(dbPath: string, filter?: TaskStatus): RustResponse<{ tasks: DownloadTask[] }>;

  /**
   * Pause a download.
   *
   * @param dbPath - Path to SQLite database
   * @param taskId - Task ID to pause
   * @returns Success status
   */
  pauseDownload(dbPath: string, taskId: string): RustResponse<{ success: boolean }>;

  /**
   * Resume a paused download.
   *
   * @param dbPath - Path to SQLite database
   * @param taskId - Task ID to resume
   * @returns Success status
   */
  resumeDownload(dbPath: string, taskId: string): RustResponse<{ success: boolean }>;

  /**
   * Cancel a download.
   *
   * @param dbPath - Path to SQLite database
   * @param taskId - Task ID to cancel
   * @returns Success status
   */
  cancelDownload(dbPath: string, taskId: string): RustResponse<{ success: boolean }>;

  // --------------------------------------------------------------------------
  // Background Task Manager (New System)
  // --------------------------------------------------------------------------

  /**
   * Start the background task service.
   */
  startBackgroundService(): RustResponse<{ success: boolean }>;

  /**
   * Stop the background task service.
   */
  stopBackgroundService(): RustResponse<{ success: boolean }>;

  /**
   * Enqueue a download using the new system.
   */
  enqueueDownloadNew(
    asin: string,
    title: string,
    author: string | undefined,
    accountJson: string,
    outputDirectory: string,
    quality: string
  ): Promise<RustResponse<{ message: string }>>;

  /**
   * Start library sync using the new system.
   */
  startLibrarySyncNew(fullSync: boolean): Promise<RustResponse<{ message: string }>>;

  /**
   * Enable automatic downloads.
   */
  enableAutoDownload(): RustResponse<{ success: boolean }>;

  /**
   * Disable automatic downloads.
   */
  disableAutoDownload(): RustResponse<{ success: boolean }>;

  /**
   * Enable automatic library sync.
   */
  enableAutoSync(intervalHours?: number): RustResponse<{ success: boolean; data?: { intervalHours: number } }>;

  /**
   * Disable automatic library sync.
   */
  disableAutoSync(): RustResponse<{ success: boolean }>;

  /**
   * Pause a task.
   */
  pauseTask(taskId: string): Promise<RustResponse<{ success: boolean }>>;

  /**
   * Resume a task.
   */
  resumeTask(taskId: string): Promise<RustResponse<{ success: boolean }>>;

  /**
   * Cancel a task.
   */
  cancelTask(taskId: string): Promise<RustResponse<{ success: boolean }>>;

  /**
   * Get all active tasks.
   */
  getActiveTasks(): RustResponse<BackgroundTask[]>;

  /**
   * Get a specific task by ID.
   */
  getTask(taskId: string): RustResponse<BackgroundTask | null>;

  /**
   * Clear all tasks (for debugging/recovery from stuck states).
   */
  clearAllTasks(): RustResponse<{ success: boolean }>;

  /**
   * Check if the background service is currently running.
   */
  isBackgroundServiceRunning(): RustResponse<{ isRunning: boolean }>;

  // --------------------------------------------------------------------------
  // Account Storage (SQLite)
  // --------------------------------------------------------------------------

  /**
   * Save account to SQLite database (single source of truth).
   */
  saveAccount(dbPath: string, accountJson: string): Promise<RustResponse<{ saved: boolean }>>;

  /**
   * Get primary account from SQLite database.
   */
  getPrimaryAccount(dbPath: string): Promise<RustResponse<{ account: string | null }>>;

  /**
   * Clear download state for all books (for testing).
   * Resets download status but keeps book metadata.
   */
  clearDownloadState(dbPath: string): Promise<RustResponse<{ books_updated: number }>>;

  /**
   * Clear all library data (for testing).
   */
  clearLibrary(dbPath: string): Promise<RustResponse<{ deleted: boolean }>>;

  // --------------------------------------------------------------------------
  // Periodic Worker Scheduling
  // --------------------------------------------------------------------------

  /**
   * Schedule periodic token refresh worker.
   *
   * @param intervalHours - Refresh interval in hours (typically 12)
   */
  scheduleTokenRefresh(intervalHours: number): RustResponse<{ success: boolean }>;

  /**
   * Schedule periodic library sync worker.
   *
   * @param intervalHours - Sync interval in hours (1, 6, 12, or 24)
   * @param wifiOnly - Whether to only sync on Wi-Fi
   */
  scheduleLibrarySync(intervalHours: number, wifiOnly: boolean): RustResponse<{ success: boolean }>;

  /**
   * Cancel token refresh worker.
   */
  cancelTokenRefresh(): RustResponse<{ success: boolean }>;

  /**
   * Cancel library sync worker.
   */
  cancelLibrarySync(): RustResponse<{ success: boolean }>;

  /**
   * Cancel all background workers.
   */
  cancelAllBackgroundTasks(): RustResponse<{ success: boolean }>;

  /**
   * Get status of token refresh worker.
   */
  getTokenRefreshStatus(): RustResponse<{ state: string }>;

  /**
   * Get status of library sync worker.
   */
  getLibrarySyncStatus(): RustResponse<{ state: string }>;
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
 * Get books with advanced filtering, sorting, and search.
 *
 * @param dbPath - Path to database file
 * @param offset - Pagination offset
 * @param limit - Maximum number of records to return
 * @param searchQuery - Optional search query (searches title, author, narrator)
 * @param seriesName - Optional series filter
 * @param category - Optional category/genre filter
 * @param sortField - Sort field: "title" | "release_date" | "date_added"
 * @param sortDirection - Sort direction: "asc" | "desc"
 * @returns Books and total count
 */
function getBooksWithFilters(
  dbPath: string,
  offset: number,
  limit: number,
  searchQuery?: string | null,
  seriesName?: string | null,
  category?: string | null,
  sortField?: string | null,
  sortDirection?: string | null
): { books: Book[]; total_count: number } {
  const response = NativeModule!.getBooksWithFilters(
    dbPath,
    offset,
    limit,
    searchQuery || null,
    seriesName || null,
    category || null,
    sortField || null,
    sortDirection || null
  );
  return unwrapResult(response);
}

/**
 * Get all unique series names from the library.
 *
 * @param dbPath - Path to database file
 * @returns Array of series names
 */
function getAllSeries(dbPath: string): string[] {
  const response = NativeModule!.getAllSeries(dbPath);
  return unwrapResult(response).series;
}

/**
 * Get all unique categories/genres from the library.
 *
 * @param dbPath - Path to database file
 * @returns Array of category names
 */
function getAllCategories(dbPath: string): string[] {
  const response = NativeModule!.getAllCategories(dbPath);
  return unwrapResult(response).categories;
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

/**
 * Download and decrypt an audiobook (complete pipeline).
 *
 * This is a high-level helper that handles the entire download+decrypt workflow:
 * 1. Request download license from Audible
 * 2. Download encrypted AAXC file
 * 3. Extract decryption keys
 * 4. Decrypt to playable M4B using FFmpeg-Kit
 * 5. Get duration and metadata
 * 6. Delete encrypted file (save space)
 *
 * @param account - Account with authentication
 * @param asin - Book ASIN to download
 * @param outputDirectory - Directory to save M4B file
 * @param quality - Download quality (defaults to "High")
 * @returns Download result with file path, size, and duration
 * @throws {RustBridgeError} If download or decryption fails
 *
 * @example
 * ```typescript
 * const result = await downloadAndDecryptBook(
 *   account,
 *   'B07T2F8VJM',
 *   '/storage/audiobooks',
 *   'High'
 * );
 * console.log(`Downloaded to: ${result.outputPath}`);
 * console.log(`Duration: ${result.duration / 3600} hours`);
 * ```
 */
async function downloadAndDecryptBook(
  account: Account,
  asin: string,
  outputDirectory: string,
  quality: string = 'High',
  dbPath: string
): Promise<{ outputPath: string; fileSize: number; duration: number }> {
  const accountJson = JSON.stringify(account);

  // Kotlin handles the entire pipeline:
  // 1. Rust downloads encrypted .aax to cache and fetches metadata from db
  // 2. Kotlin decrypts with FFmpeg-Kit and embeds metadata (title, author, narrator, ASIN, etc.)
  // 3. Kotlin copies to user's directory (if SAF URI)
  // 4. Kotlin cleans up cache files
  const response = await NativeModule!.downloadBook(
    accountJson,
    asin,
    outputDirectory,
    quality,
    dbPath
  );

  return unwrapResult(response);
}

/**
 * Enqueue a download using the persistent download manager.
 *
 * This starts a background download that can be paused, resumed, and monitored.
 * The download continues even if the app is backgrounded.
 *
 * @param dbPath - Path to database file
 * @param account - Account with authentication
 * @param asin - Book ASIN to download
 * @param title - Book title
 * @param outputDirectory - Directory to save file (supports SAF URIs)
 * @param quality - Download quality (defaults to "High")
 * @throws {RustBridgeError} If enqueue fails
 *
 * @example
 * ```typescript
 * await enqueueDownload(dbPath, account, 'B07NP9L44Y', 'A Mind of Her Own', downloadDir);
 * ```
 */
async function enqueueDownload(
  dbPath: string,
  account: Account,
  asin: string,
  title: string,
  outputDirectory: string,
  quality: string = 'High'
): Promise<void> {
  const accountJson = JSON.stringify(account);

  const response = await NativeModule!.enqueueDownload(
    dbPath,
    accountJson,
    asin,
    title,
    outputDirectory,
    quality
  );

  unwrapResult(response);
}

/**
 * Get download task status.
 *
 * @param dbPath - Path to database file
 * @param taskId - Task ID
 * @returns Task details
 */
function getDownloadTask(dbPath: string, taskId: string): DownloadTask {
  const response = NativeModule!.getDownloadTask(dbPath, taskId);
  return unwrapResult(response);
}

/**
 * List all download tasks with optional filter.
 *
 * @param dbPath - Path to database file
 * @param filter - Optional status filter
 * @returns Array of tasks
 */
function listDownloadTasks(dbPath: string, filter?: TaskStatus): DownloadTask[] {
  const response = NativeModule!.listDownloadTasks(dbPath, filter);
  const data = unwrapResult(response);
  return data.tasks;
}

/**
 * Pause a download.
 *
 * @param dbPath - Path to database file
 * @param taskId - Task ID to pause
 */
function pauseDownload(dbPath: string, taskId: string): void {
  const response = NativeModule!.pauseDownload(dbPath, taskId);
  unwrapResult(response);
}

/**
 * Resume a paused download.
 *
 * @param dbPath - Path to database file
 * @param taskId - Task ID to resume
 */
function resumeDownload(dbPath: string, taskId: string): void {
  const response = NativeModule!.resumeDownload(dbPath, taskId);
  unwrapResult(response);
}

/**
 * Cancel a download.
 *
 * @param dbPath - Path to database file
 * @param taskId - Task ID to cancel
 */
function cancelDownload(dbPath: string, taskId: string): void {
  const response = NativeModule!.cancelDownload(dbPath, taskId);
  unwrapResult(response);
}

// ============================================================================
// Background Task Manager (New System)
// ============================================================================

/**
 * Task types in the background task system.
 */
export type BackgroundTaskType = 'DOWNLOAD' | 'TOKEN_REFRESH' | 'LIBRARY_SYNC' | 'AUTO_DOWNLOAD';

/**
 * Task priorities.
 */
export type TaskPriority = 'CRITICAL' | 'HIGH' | 'MEDIUM' | 'LOW';

/**
 * Task statuses.
 */
export type BackgroundTaskStatus = 'PENDING' | 'RUNNING' | 'PAUSED' | 'COMPLETED' | 'FAILED' | 'CANCELLED';

/**
 * Background task representation.
 */
export interface BackgroundTask {
  id: string;
  type: BackgroundTaskType;
  priority: TaskPriority;
  status: BackgroundTaskStatus;
  metadata: Record<string, any>;
  createdAt: number;
  startedAt?: number;
  completedAt?: number;
  error?: string;
}

/**
 * Start the background task service.
 * Must be called once when app starts.
 */
function startBackgroundService(): void {
  const response = NativeModule!.startBackgroundService();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to start background service');
  }
}

/**
 * Stop the background task service.
 * This disables all automatic features (token refresh, library sync, auto-download).
 */
function stopBackgroundService(): void {
  const response = NativeModule!.stopBackgroundService();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to stop background service');
  }
}

/**
 * Enqueue a download using the new background task system.
 *
 * @param asin - Book ASIN
 * @param title - Book title
 * @param author - Optional book author
 * @param account - Account with authentication
 * @param outputDirectory - Output directory (SAF URI)
 * @param quality - Download quality
 */
async function enqueueDownloadNew(
  asin: string,
  title: string,
  author: string | undefined,
  account: Account,
  outputDirectory: string,
  quality: string = 'High'
): Promise<void> {
  const accountJson = JSON.stringify(account);

  const response = await NativeModule!.enqueueDownloadNew(
    asin,
    title,
    author,
    accountJson,
    outputDirectory,
    quality
  );

  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to enqueue download');
  }
}

/**
 * Start library sync using the new background task system.
 *
 * @param fullSync - Whether to do a full sync (default: false)
 */
async function startLibrarySyncNew(fullSync: boolean = false): Promise<void> {
  const response = await NativeModule!.startLibrarySyncNew(fullSync);
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to start library sync');
  }
}

/**
 * Enable automatic downloads after library sync.
 */
function enableAutoDownload(): void {
  const response = NativeModule!.enableAutoDownload();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to enable auto-download');
  }
}

/**
 * Disable automatic downloads.
 */
function disableAutoDownload(): void {
  const response = NativeModule!.disableAutoDownload();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to disable auto-download');
  }
}

/**
 * Enable automatic library sync.
 *
 * @param intervalHours - Sync interval in hours (default: 24)
 */
function enableAutoSync(intervalHours?: number): void {
  const response = NativeModule!.enableAutoSync(intervalHours);
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to enable auto-sync');
  }
}

/**
 * Disable automatic library sync.
 */
function disableAutoSync(): void {
  const response = NativeModule!.disableAutoSync();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to disable auto-sync');
  }
}

/**
 * Pause a task.
 *
 * @param taskId - Task ID to pause
 */
async function pauseTask(taskId: string): Promise<boolean> {
  const response = await NativeModule!.pauseTask(taskId);
  return response.success || false;
}

/**
 * Resume a paused task.
 *
 * @param taskId - Task ID to resume
 */
async function resumeTask(taskId: string): Promise<boolean> {
  const response = await NativeModule!.resumeTask(taskId);
  return response.success || false;
}

/**
 * Cancel a task.
 *
 * @param taskId - Task ID to cancel
 */
async function cancelTask(taskId: string): Promise<boolean> {
  const response = await NativeModule!.cancelTask(taskId);
  return response.success || false;
}

/**
 * Get all active tasks.
 *
 * @returns Array of active tasks
 */
function getActiveTasks(): BackgroundTask[] {
  const response = NativeModule!.getActiveTasks();
  if (!response.success || !response.data) {
    return [];
  }
  return response.data as BackgroundTask[];
}

/**
 * Get a specific task by ID.
 *
 * @param taskId - Task ID
 * @returns Task details or null if not found
 */
function getTask(taskId: string): BackgroundTask | null {
  const response = NativeModule!.getTask(taskId);
  if (!response.success || !response.data) {
    return null;
  }
  return response.data as BackgroundTask;
}

/**
 * Clear all tasks (for debugging/recovery from stuck states).
 *
 * This removes all active tasks from the task manager. Use this to recover
 * from stuck states or clear completed/failed tasks.
 */
function clearAllTasks(): void {
  const response = NativeModule!.clearAllTasks();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to clear all tasks');
  }
}

/**
 * Check if the background service is currently running.
 *
 * @returns true if the service is running, false otherwise
 */
function isBackgroundServiceRunning(): boolean {
  const response = NativeModule!.isBackgroundServiceRunning();
  if (!response.success || !response.data) {
    return false;
  }
  return response.data.isRunning;
}

/**
 * Save account to SQLite database (single source of truth).
 *
 * @param dbPath - Database path
 * @param account - Account object
 */
async function saveAccount(dbPath: string, account: Account): Promise<void> {
  const accountJson = JSON.stringify(account);
  const response = await NativeModule!.saveAccount(dbPath, accountJson);
  unwrapResult(response);
}

/**
 * Get primary account from SQLite database.
 *
 * @param dbPath - Database path
 * @returns Account object or null if no account exists
 */
async function getPrimaryAccount(dbPath: string): Promise<Account | null> {
  const response = await NativeModule!.getPrimaryAccount(dbPath);
  const data = unwrapResult(response);

  if (!data.account || data.account === 'null') {
    return null;
  }

  return JSON.parse(data.account);
}

/**
 * Clear all library data (for testing).
 *
 * @param dbPath - Database path
 */
async function clearDownloadState(dbPath: string): Promise<number> {
  const response = await NativeModule!.clearDownloadState(dbPath);
  const data = unwrapResult(response);
  return data.books_updated;
}

async function clearLibrary(dbPath: string): Promise<void> {
  const response = await NativeModule!.clearLibrary(dbPath);
  unwrapResult(response);
}

// ============================================================================
// Periodic Worker Scheduling
// ============================================================================

/**
 * Schedule periodic token refresh worker.
 *
 * @param intervalHours - Refresh interval in hours (typically 12)
 */
function scheduleTokenRefresh(intervalHours: number): void {
  const response = NativeModule!.scheduleTokenRefresh(intervalHours);
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to schedule token refresh');
  }
}

/**
 * Schedule periodic library sync worker.
 *
 * @param intervalHours - Sync interval in hours (1, 6, 12, or 24)
 * @param wifiOnly - Whether to only sync on Wi-Fi
 */
function scheduleLibrarySync(intervalHours: number, wifiOnly: boolean): void {
  const response = NativeModule!.scheduleLibrarySync(intervalHours, wifiOnly);
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to schedule library sync');
  }
}

/**
 * Cancel token refresh worker.
 */
function cancelTokenRefresh(): void {
  const response = NativeModule!.cancelTokenRefresh();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to cancel token refresh');
  }
}

/**
 * Cancel library sync worker.
 */
function cancelLibrarySync(): void {
  const response = NativeModule!.cancelLibrarySync();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to cancel library sync');
  }
}

/**
 * Cancel all background workers.
 */
function cancelAllBackgroundTasks(): void {
  const response = NativeModule!.cancelAllBackgroundTasks();
  if (!response.success) {
    throw new RustBridgeError(response.error || 'Failed to cancel all background tasks');
  }
}

/**
 * Get status of token refresh worker.
 *
 * @returns Worker state (NOT_SCHEDULED, ENQUEUED, RUNNING, SUCCEEDED, FAILED, CANCELLED)
 */
function getTokenRefreshStatus(): string {
  const response = NativeModule!.getTokenRefreshStatus();
  if (!response.success || !response.data) {
    return 'NOT_SCHEDULED';
  }
  return response.data.state;
}

/**
 * Get status of library sync worker.
 *
 * @returns Worker state (NOT_SCHEDULED, ENQUEUED, RUNNING, SUCCEEDED, FAILED, CANCELLED)
 */
function getLibrarySyncStatus(): string {
  const response = NativeModule!.getLibrarySyncStatus();
  if (!response.success || !response.data) {
    return 'NOT_SCHEDULED';
  }
  return response.data.state;
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
  getBooksWithFilters,
  getAllSeries,
  getAllCategories,
  getCustomerInformation,
  downloadAndDecryptBook,
  generateDeviceSerial,
  unwrapResult,
  RustBridgeError,
  // Download Manager (Old System)
  enqueueDownload,
  getDownloadTask,
  listDownloadTasks,
  pauseDownload,
  resumeDownload,
  cancelDownload,
  // Background Task Manager (New System)
  startBackgroundService,
  stopBackgroundService,
  enqueueDownloadNew,
  startLibrarySyncNew,
  enableAutoDownload,
  disableAutoDownload,
  enableAutoSync,
  disableAutoSync,
  pauseTask,
  resumeTask,
  cancelTask,
  getActiveTasks,
  getTask,
  clearAllTasks,
  isBackgroundServiceRunning,
  // Account Storage (SQLite)
  saveAccount,
  getPrimaryAccount,
  // Testing
  clearDownloadState,
  clearLibrary,
  // Periodic Worker Scheduling
  scheduleTokenRefresh,
  scheduleLibrarySync,
  cancelTokenRefresh,
  cancelLibrarySync,
  cancelAllBackgroundTasks,
  getTokenRefreshStatus,
  getLibrarySyncStatus,
};
