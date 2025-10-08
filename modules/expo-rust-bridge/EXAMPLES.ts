/**
 * Expo Rust Bridge - Code Examples
 *
 * This file contains working examples of using the Rust bridge.
 * These examples can be copied and adapted for your use cases.
 */

import ExpoRustBridge, {
  initiateOAuth,
  completeOAuthFlow,
  refreshToken,
  getActivationBytes,
  initializeDatabase,
  syncLibrary,
  unwrapResult,
  RustBridgeError,
} from './index';

import type {
  Account,
  Book,
  Locale,
  TokenResponse,
  RegistrationResponse,
  SyncStats,
  OAuthFlowData,
} from './index';

// ============================================================================
// Example 1: Simple Bridge Test
// ============================================================================

export function testBridgeConnection(): void {
  try {
    const response = ExpoRustBridge!.testBridge();
    const data = unwrapResult(response);

    console.log('Bridge test results:');
    console.log('  Bridge active:', data.bridgeActive);
    console.log('  Rust loaded:', data.rustLoaded);
    console.log('  Version:', data.version);
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Bridge test failed:', error.rustError);
    }
    throw error;
  }
}

// ============================================================================
// Example 2: OAuth Authentication Flow
// ============================================================================

export class AuthenticationManager {
  private oauthFlowData: OAuthFlowData | null = null;
  private currentLocale: string | null = null;

  /**
   * Step 1: Get available locales and display to user
   */
  getAvailableLocales(): Locale[] {
    const response = ExpoRustBridge!.getSupportedLocales();
    const { locales } = unwrapResult(response);
    return locales;
  }

  /**
   * Step 2: Start OAuth flow with selected locale
   */
  startOAuthFlow(localeCode: string): string {
    try {
      this.currentLocale = localeCode;
      this.oauthFlowData = initiateOAuth(localeCode);

      console.log('OAuth URL generated:', this.oauthFlowData.url);
      console.log('Device serial:', this.oauthFlowData.deviceSerial);

      // Return URL to open in WebView
      return this.oauthFlowData.url;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Failed to start OAuth:', error.rustError);
      }
      throw error;
    }
  }

  /**
   * Step 3: Complete OAuth flow after callback
   */
  async completeOAuthFlow(callbackUrl: string): Promise<RegistrationResponse> {
    if (!this.oauthFlowData || !this.currentLocale) {
      throw new Error('OAuth flow not initiated');
    }

    try {
      const tokens = await completeOAuthFlow(
        callbackUrl,
        this.currentLocale,
        this.oauthFlowData.deviceSerial,
        this.oauthFlowData.pkceVerifier
      );

      console.log('Authentication successful');
      console.log('Access token:', tokens.bearer.access_token.substring(0, 20) + '...');
      console.log('Expires in:', tokens.bearer.expires_in, 'seconds');

      return tokens;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('OAuth completion failed:', error.rustError);
      }
      throw error;
    } finally {
      // Clean up
      this.oauthFlowData = null;
      this.currentLocale = null;
    }
  }

  /**
   * Step 4: Get activation bytes for DRM removal
   */
  async getActivationBytes(account: Account): Promise<string> {
    try {
      const activationBytes = await getActivationBytes(account);

      console.log('Activation bytes retrieved:', activationBytes);

      return activationBytes;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Failed to get activation bytes:', error.rustError);
      }
      throw error;
    }
  }

  /**
   * Step 5: Refresh expired access token
   */
  async refreshAccessToken(account: Account): Promise<TokenResponse> {
    try {
      const newTokens = await refreshToken(account);

      console.log('Token refreshed successfully');

      return newTokens;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Token refresh failed:', error.rustError);
      }
      throw error;
    }
  }
}

// ============================================================================
// Example 3: Complete Authentication Flow
// ============================================================================

export async function completeAuthenticationExample(
  localeCode: string,
  callbackUrl: string
): Promise<Account> {
  const authManager = new AuthenticationManager();

  try {
    // Get locales (optional - for display purposes)
    const locales = authManager.getAvailableLocales();
    console.log('Available locales:', locales.map(l => l.country_code).join(', '));

    // Start OAuth flow
    const oauthUrl = authManager.startOAuthFlow(localeCode);
    console.log('Open this URL in WebView:', oauthUrl);

    // After user completes OAuth in WebView, handle callback
    const tokens = await authManager.completeOAuthFlow(callbackUrl);

    // Find selected locale
    const locale = locales.find(l => l.country_code === localeCode);
    if (!locale) {
      throw new Error('Locale not found');
    }

    // Create account object
    const account: Account = {
      account_id: 'unique-account-id',
      account_name: 'My Audible Account',
      locale: locale,
      identity: {
        access_token: {
          token: tokens.bearer.access_token,
          expires_at: new Date(Date.now() + parseInt(tokens.bearer.expires_in) * 1000).toISOString(),
        },
        refresh_token: tokens.bearer.refresh_token,
        device_private_key: tokens.mac_dms.device_private_key,
        adp_token: tokens.mac_dms.adp_token,
        cookies: {},
        device_serial_number: tokens.device_info.device_serial_number,
        device_type: tokens.device_info.device_type,
        device_name: tokens.device_info.device_name,
        amazon_account_id: tokens.customer_info.user_id,
        store_authentication_cookie: tokens.store_authentication_cookie.cookie,
        locale: locale,
        customer_info: tokens.customer_info,
      },
    };

    // Get activation bytes for DRM removal
    const activationBytes = await authManager.getActivationBytes(account);
    account.decrypt_key = activationBytes;

    console.log('Authentication complete!');
    return account;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Authentication failed:', error.rustError);
    }
    throw error;
  }
}

// ============================================================================
// Example 4: Database Operations
// ============================================================================

export class LibraryManager {
  private dbPath: string;

  constructor(dbPath: string) {
    this.dbPath = dbPath;
    this.initializeDatabase();
  }

  /**
   * Initialize database (creates schema if not exists)
   */
  private initializeDatabase(): void {
    try {
      initializeDatabase(this.dbPath);
      console.log('Database initialized at:', this.dbPath);
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Database initialization failed:', error.rustError);
      }
      throw error;
    }
  }

  /**
   * Sync library from Audible API
   */
  async syncFromAudible(account: Account): Promise<SyncStats> {
    try {
      console.log('Starting library sync...');

      const stats = await syncLibrary(this.dbPath, account);

      console.log('Sync complete:');
      console.log('  Total items:', stats.total_items);
      console.log('  Books added:', stats.books_added);
      console.log('  Books updated:', stats.books_updated);
      console.log('  Books absent:', stats.books_absent);

      if (stats.errors.length > 0) {
        console.warn('Sync errors:', stats.errors);
      }

      return stats;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Library sync failed:', error.rustError);
      }
      throw error;
    }
  }

  /**
   * Get books with pagination
   */
  getBooks(page: number = 0, pageSize: number = 20): Book[] {
    try {
      const offset = page * pageSize;
      const response = ExpoRustBridge!.getBooks(this.dbPath, offset, pageSize);
      const { books } = unwrapResult(response);

      console.log(`Retrieved ${books.length} books (page ${page})`);

      return books;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Failed to get books:', error.rustError);
      }
      throw error;
    }
  }

  /**
   * Search books by title, author, or narrator
   */
  searchBooks(query: string): Book[] {
    try {
      const response = ExpoRustBridge!.searchBooks(this.dbPath, query);
      const { books } = unwrapResult(response);

      console.log(`Found ${books.length} books matching "${query}"`);

      return books;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Search failed:', error.rustError);
      }
      throw error;
    }
  }
}

// ============================================================================
// Example 5: Download and Decrypt
// ============================================================================

export class AudiobookManager {
  /**
   * Download audiobook from Audible
   */
  async downloadBook(
    asin: string,
    license: any,
    outputPath: string
  ): Promise<string> {
    try {
      console.log(`Downloading book ${asin}...`);

      const licenseJson = JSON.stringify(license);
      const response = await ExpoRustBridge!.downloadBook(
        asin,
        licenseJson,
        outputPath
      );

      const { file_path } = unwrapResult(response);

      console.log(`Download complete: ${file_path}`);
      return file_path;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Download failed:', error.rustError);
      }
      throw error;
    }
  }

  /**
   * Decrypt AAX file to M4B
   */
  async decryptBook(
    inputPath: string,
    outputPath: string,
    activationBytes: string
  ): Promise<string> {
    try {
      // Validate activation bytes first
      const validationResponse = ExpoRustBridge!.validateActivationBytes(activationBytes);
      const { valid } = unwrapResult(validationResponse);

      if (!valid) {
        throw new Error('Invalid activation bytes format');
      }

      console.log(`Decrypting ${inputPath}...`);

      const response = await ExpoRustBridge!.decryptAAX(
        inputPath,
        outputPath,
        activationBytes
      );

      const { output_path } = unwrapResult(response);

      console.log(`Decryption complete: ${output_path}`);
      return output_path;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Decryption failed:', error.rustError);
      }
      throw error;
    }
  }

  /**
   * Complete download and decrypt workflow
   */
  async downloadAndDecrypt(
    asin: string,
    license: any,
    downloadPath: string,
    outputPath: string,
    activationBytes: string
  ): Promise<string> {
    try {
      // Download encrypted file
      const aaxPath = await this.downloadBook(asin, license, downloadPath);

      // Decrypt to M4B
      const m4bPath = await this.decryptBook(aaxPath, outputPath, activationBytes);

      return m4bPath;
    } catch (error) {
      console.error('Download and decrypt failed:', error);
      throw error;
    }
  }
}

// ============================================================================
// Example 6: Error Handling Patterns
// ============================================================================

/**
 * Pattern 1: Simple try-catch
 */
export function simpleErrorHandling(): void {
  try {
    const response = ExpoRustBridge!.testBridge();
    const data = unwrapResult(response);
    console.log('Success:', data);
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Rust error:', error.rustError);
    } else {
      console.error('Unexpected error:', error);
    }
  }
}

/**
 * Pattern 2: Async error handling
 */
export async function asyncErrorHandling(account: Account): Promise<void> {
  try {
    const activationBytes = await getActivationBytes(account);
    console.log('Activation bytes:', activationBytes);
  } catch (error) {
    if (error instanceof RustBridgeError) {
      // Check specific error types
      if (error.rustError?.includes('authentication')) {
        console.error('Authentication error - token may be expired');
      } else if (error.rustError?.includes('network')) {
        console.error('Network error - check connection');
      } else {
        console.error('Unknown Rust error:', error.rustError);
      }
    }
    throw error;
  }
}

/**
 * Pattern 3: Custom error mapper
 */
export function mapRustError(error: unknown): string {
  if (error instanceof RustBridgeError) {
    const rustError = error.rustError || error.message;

    // Map Rust errors to user-friendly messages
    if (rustError.includes('authentication') || rustError.includes('token')) {
      return 'Your session has expired. Please log in again.';
    }
    if (rustError.includes('network') || rustError.includes('connection')) {
      return 'Unable to connect. Please check your internet connection.';
    }
    if (rustError.includes('database') || rustError.includes('sql')) {
      return 'Database error. Please try restarting the app.';
    }
    if (rustError.includes('file') || rustError.includes('io')) {
      return 'File access error. Please check storage permissions.';
    }
    if (rustError.includes('decrypt') || rustError.includes('activation')) {
      return 'Decryption failed. Please check your activation bytes.';
    }

    return rustError;
  }

  return 'An unexpected error occurred.';
}

// ============================================================================
// Example 7: Complete Usage Workflow
// ============================================================================

export async function completeWorkflowExample(
  localeCode: string,
  callbackUrl: string,
  dbPath: string
): Promise<void> {
  console.log('=== Starting Complete Workflow ===');

  try {
    // Step 1: Authenticate
    console.log('\n1. Authenticating...');
    const account = await completeAuthenticationExample(localeCode, callbackUrl);

    // Step 2: Initialize database
    console.log('\n2. Initializing database...');
    const library = new LibraryManager(dbPath);

    // Step 3: Sync library
    console.log('\n3. Syncing library...');
    const syncStats = await library.syncFromAudible(account);
    console.log(`Synced ${syncStats.books_added} new books`);

    // Step 4: Load books
    console.log('\n4. Loading books...');
    const books = library.getBooks(0, 10);
    console.log(`Loaded ${books.length} books`);

    // Step 5: Search books
    console.log('\n5. Searching books...');
    const searchResults = library.searchBooks('harry potter');
    console.log(`Found ${searchResults.length} matching books`);

    // Step 6: Download and decrypt (example for first book)
    if (books.length > 0 && account.decrypt_key) {
      console.log('\n6. Processing audiobook...');
      const book = books[0];
      const audiobookManager = new AudiobookManager();

      // Note: You would need actual license data from Audible API
      const license = { /* license data */ };

      const m4bPath = await audiobookManager.downloadAndDecrypt(
        book.audible_product_id,
        license,
        `/path/to/downloads/${book.audible_product_id}.aax`,
        `/path/to/output/${book.audible_product_id}.m4b`,
        account.decrypt_key
      );

      console.log(`Book processed: ${m4bPath}`);
    }

    console.log('\n=== Workflow Complete ===');
  } catch (error) {
    console.error('\n!!! Workflow Failed !!!');
    console.error('Error:', mapRustError(error));
    throw error;
  }
}

// ============================================================================
// Example 8: React Hook for Library Management
// ============================================================================

/**
 * Example React hook (requires React Native environment)
 * Uncomment when using in React Native app
 */
/*
import { useState, useEffect, useCallback } from 'react';

export function useLibrary(dbPath: string, account: Account | null) {
  const [books, setBooks] = useState<Book[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [syncStats, setSyncStats] = useState<SyncStats | null>(null);

  const library = useMemo(() => new LibraryManager(dbPath), [dbPath]);

  const sync = useCallback(async () => {
    if (!account) {
      setError('No account available');
      return;
    }

    setLoading(true);
    setError(null);

    try {
      const stats = await library.syncFromAudible(account);
      setSyncStats(stats);

      // Reload books after sync
      const newBooks = library.getBooks(0, 20);
      setBooks(newBooks);
    } catch (err) {
      setError(mapRustError(err));
    } finally {
      setLoading(false);
    }
  }, [library, account]);

  const loadBooks = useCallback((page: number = 0, pageSize: number = 20) => {
    setLoading(true);
    setError(null);

    try {
      const loadedBooks = library.getBooks(page, pageSize);
      setBooks(loadedBooks);
    } catch (err) {
      setError(mapRustError(err));
    } finally {
      setLoading(false);
    }
  }, [library]);

  const search = useCallback((query: string) => {
    setLoading(true);
    setError(null);

    try {
      const results = library.searchBooks(query);
      setBooks(results);
    } catch (err) {
      setError(mapRustError(err));
    } finally {
      setLoading(false);
    }
  }, [library]);

  // Load initial books
  useEffect(() => {
    loadBooks();
  }, [loadBooks]);

  return {
    books,
    loading,
    error,
    syncStats,
    sync,
    loadBooks,
    search,
  };
}
*/
