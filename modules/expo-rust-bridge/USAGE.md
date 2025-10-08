# Expo Rust Bridge - Usage Guide

This document provides comprehensive documentation for using the TypeScript interface to the native Rust bridge.

## Table of Contents

- [Installation](#installation)
- [Basic Usage](#basic-usage)
- [Authentication Flow](#authentication-flow)
- [Database Operations](#database-operations)
- [Download & Decryption](#download--decryption)
- [Error Handling](#error-handling)
- [Type Reference](#type-reference)
- [Complete Examples](#complete-examples)

## Installation

The Expo Rust Bridge is already integrated into the project. Before using it, ensure native libraries are built:

```bash
# For Android
npm run build:rust:android

# For iOS
npm run build:rust:ios

# For both platforms
npm run build:rust
```

## Basic Usage

### Importing the Module

```typescript
// Import the native module directly
import ExpoRustBridge from '../modules/expo-rust-bridge';

// Or use named imports for helper functions
import {
  initiateOAuth,
  completeOAuthFlow,
  unwrapResult,
  RustBridgeError
} from '../modules/expo-rust-bridge';

// Import types
import type {
  Account,
  Book,
  TokenResponse,
  SyncStats
} from '../modules/expo-rust-bridge';
```

### Testing the Bridge

```typescript
try {
  const result = ExpoRustBridge.testBridge();
  console.log('Bridge active:', result.data?.bridgeActive);
  console.log('Rust loaded:', result.data?.rustLoaded);
  console.log('Version:', result.data?.version);
} catch (error) {
  console.error('Bridge test failed:', error);
}
```

## Authentication Flow

### Step 1: Get Supported Locales

```typescript
import { unwrapResult } from '../modules/expo-rust-bridge';
import ExpoRustBridge from '../modules/expo-rust-bridge';

function getSupportedLocales() {
  const response = ExpoRustBridge.getSupportedLocales();
  const { locales } = unwrapResult(response);

  return locales; // Array of { country_code, name, domain }
}
```

### Step 2: Initiate OAuth Flow

```typescript
import { initiateOAuth } from '../modules/expo-rust-bridge';

async function startLogin(localeCode: string) {
  try {
    // Generate OAuth URL with PKCE
    const flowData = initiateOAuth(localeCode);

    // Store these values for later use
    const { url, pkceVerifier, state, deviceSerial } = flowData;

    // Store in state or async storage
    await AsyncStorage.setItem('oauth_pkce_verifier', pkceVerifier);
    await AsyncStorage.setItem('oauth_device_serial', deviceSerial);
    await AsyncStorage.setItem('oauth_state', state);

    // Open OAuth URL in WebView
    return url;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('OAuth initiation failed:', error.rustError);
    }
    throw error;
  }
}
```

### Step 3: Handle OAuth Callback

```typescript
import { completeOAuthFlow } from '../modules/expo-rust-bridge';

async function handleOAuthCallback(callbackUrl: string, localeCode: string) {
  try {
    // Retrieve stored values
    const pkceVerifier = await AsyncStorage.getItem('oauth_pkce_verifier');
    const deviceSerial = await AsyncStorage.getItem('oauth_device_serial');

    if (!pkceVerifier || !deviceSerial) {
      throw new Error('OAuth flow state not found');
    }

    // Complete the flow
    const tokens = await completeOAuthFlow(
      callbackUrl,
      localeCode,
      deviceSerial,
      pkceVerifier
    );

    // Store tokens securely
    await AsyncStorage.setItem('access_token', tokens.access_token);
    await AsyncStorage.setItem('refresh_token', tokens.refresh_token);

    // Clean up temporary OAuth data
    await AsyncStorage.multiRemove([
      'oauth_pkce_verifier',
      'oauth_device_serial',
      'oauth_state'
    ]);

    return tokens;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('OAuth completion failed:', error.rustError);
    }
    throw error;
  }
}
```

### Step 4: Get Activation Bytes

```typescript
import { getActivationBytes } from '../modules/expo-rust-bridge';
import type { Account } from '../modules/expo-rust-bridge';

async function fetchActivationBytes(account: Account) {
  try {
    const activationBytes = await getActivationBytes(account);

    // Store activation bytes in account
    account.decrypt_key = activationBytes;

    return activationBytes;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Failed to get activation bytes:', error.rustError);
    }
    throw error;
  }
}
```

### Step 5: Refresh Tokens

```typescript
import { refreshToken } from '../modules/expo-rust-bridge';
import type { Account } from '../modules/expo-rust-bridge';

async function refreshAccountTokens(account: Account) {
  try {
    const newTokens = await refreshToken(account);

    // Update account with new tokens
    if (account.identity) {
      account.identity.access_token = newTokens.access_token;
      account.identity.refresh_token = newTokens.refresh_token;

      // Calculate new expiration time
      const expiresAt = new Date();
      expiresAt.setSeconds(expiresAt.getSeconds() + newTokens.expires_in);
      account.identity.expires_at = expiresAt.toISOString();
    }

    return account;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Token refresh failed:', error.rustError);
    }
    throw error;
  }
}
```

## Database Operations

### Initialize Database

```typescript
import { initializeDatabase } from '../modules/expo-rust-bridge';
import * as FileSystem from 'expo-file-system';

function setupDatabase() {
  // Get app's document directory
  const dbPath = `${FileSystem.documentDirectory}audible.db`;

  try {
    initializeDatabase(dbPath);
    console.log('Database initialized at:', dbPath);
    return dbPath;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Database initialization failed:', error.rustError);
    }
    throw error;
  }
}
```

### Sync Library from Audible

```typescript
import { syncLibrary } from '../modules/expo-rust-bridge';
import type { Account, SyncStats } from '../modules/expo-rust-bridge';

async function synchronizeLibrary(
  dbPath: string,
  account: Account
): Promise<SyncStats> {
  try {
    const stats = await syncLibrary(dbPath, account);

    console.log(`Sync complete:
      Total items: ${stats.total_items}
      Added: ${stats.books_added}
      Updated: ${stats.books_updated}
      Absent: ${stats.books_absent}
      Errors: ${stats.errors.length}
    `);

    if (stats.errors.length > 0) {
      console.error('Sync errors:', stats.errors);
    }

    return stats;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Library sync failed:', error.rustError);
    }
    throw error;
  }
}
```

### Get Books with Pagination

```typescript
import { unwrapResult } from '../modules/expo-rust-bridge';
import ExpoRustBridge from '../modules/expo-rust-bridge';
import type { Book } from '../modules/expo-rust-bridge';

function getBooks(
  dbPath: string,
  page: number = 0,
  pageSize: number = 20
): Book[] {
  const offset = page * pageSize;
  const response = ExpoRustBridge.getBooks(dbPath, offset, pageSize);
  const { books } = unwrapResult(response);
  return books;
}
```

### Search Books

```typescript
import { unwrapResult } from '../modules/expo-rust-bridge';
import ExpoRustBridge from '../modules/expo-rust-bridge';
import type { Book } from '../modules/expo-rust-bridge';

function searchBooks(dbPath: string, query: string): Book[] {
  const response = ExpoRustBridge.searchBooks(dbPath, query);
  const { books } = unwrapResult(response);
  return books;
}
```

## Download & Decryption

### Download Audiobook

```typescript
import ExpoRustBridge from '../modules/expo-rust-bridge';
import { unwrapResult } from '../modules/expo-rust-bridge';
import * as FileSystem from 'expo-file-system';

async function downloadAudiobook(
  asin: string,
  license: any, // License object from Audible API
  outputDir: string
): Promise<string> {
  try {
    const outputPath = `${outputDir}/${asin}.aax`;
    const licenseJson = JSON.stringify(license);

    const response = await ExpoRustBridge.downloadBook(
      asin,
      licenseJson,
      outputPath
    );

    const { file_path } = unwrapResult(response);
    console.log('Downloaded to:', file_path);

    return file_path;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Download failed:', error.rustError);
    }
    throw error;
  }
}
```

### Decrypt AAX to M4B

```typescript
import ExpoRustBridge from '../modules/expo-rust-bridge';
import { unwrapResult } from '../modules/expo-rust-bridge';

async function decryptAudiobook(
  inputPath: string,
  activationBytes: string,
  outputDir: string
): Promise<string> {
  try {
    const outputPath = inputPath.replace('.aax', '.m4b');

    const response = await ExpoRustBridge.decryptAAX(
      inputPath,
      outputPath,
      activationBytes
    );

    const { output_path } = unwrapResult(response);
    console.log('Decrypted to:', output_path);

    return output_path;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Decryption failed:', error.rustError);
    }
    throw error;
  }
}
```

### Validate Activation Bytes

```typescript
import { unwrapResult } from '../modules/expo-rust-bridge';
import ExpoRustBridge from '../modules/expo-rust-bridge';

function validateActivationBytes(activationBytes: string): boolean {
  const response = ExpoRustBridge.validateActivationBytes(activationBytes);
  const { valid } = unwrapResult(response);
  return valid;
}
```

## Error Handling

### Using Try-Catch

```typescript
import { RustBridgeError } from '../modules/expo-rust-bridge';

async function safeOperation() {
  try {
    // Your operation here
    const result = await someRustBridgeFunction();
    return result;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      // Handle Rust-specific errors
      console.error('Rust error:', error.rustError);
      console.error('Message:', error.message);

      // You can show user-friendly error messages
      Alert.alert('Error', error.message);
    } else {
      // Handle other errors
      console.error('Unexpected error:', error);
    }
    throw error;
  }
}
```

### Using unwrapResult

```typescript
import { unwrapResult, RustBridgeError } from '../modules/expo-rust-bridge';
import ExpoRustBridge from '../modules/expo-rust-bridge';

function directUnwrap() {
  try {
    const response = ExpoRustBridge.testBridge();
    const data = unwrapResult(response); // Throws RustBridgeError if failed
    return data;
  } catch (error) {
    if (error instanceof RustBridgeError) {
      console.error('Unwrap failed:', error.rustError);
    }
    throw error;
  }
}
```

### Custom Error Handler

```typescript
import { RustBridgeError } from '../modules/expo-rust-bridge';

function handleRustError(error: unknown): string {
  if (error instanceof RustBridgeError) {
    // Map Rust errors to user-friendly messages
    const errorMessage = error.rustError || error.message;

    if (errorMessage.includes('authentication')) {
      return 'Authentication failed. Please log in again.';
    } else if (errorMessage.includes('network')) {
      return 'Network error. Please check your connection.';
    } else if (errorMessage.includes('database')) {
      return 'Database error. Please try again.';
    }

    return errorMessage;
  }

  return 'An unexpected error occurred.';
}
```

## Type Reference

### Core Types

```typescript
// Generic response wrapper
interface RustResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

// OAuth types
interface OAuthUrlData {
  url: string;
  pkce_verifier: string;
  state: string;
  device_serial: string;
}

interface OAuthFlowData {
  url: string;
  pkceVerifier: string;
  state: string;
  deviceSerial: string;
}

interface TokenResponse {
  access_token: string;
  refresh_token: string;
  expires_in: number;
  token_type: string;
}

interface Locale {
  country_code: string;
  name: string;
  domain: string;
}

// Account types
interface Account {
  account_id: string;
  account_name?: string;
  decrypt_key?: string;
  locale: Locale;
  identity?: Identity;
}

interface Identity {
  access_token: string;
  refresh_token: string;
  expires_at: string; // ISO 8601 timestamp
  device_serial: string;
  device_type: string;
}

// Book types
interface Book {
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
}

// Sync statistics
interface SyncStats {
  total_items: number;
  books_added: number;
  books_updated: number;
  books_absent: number;
  errors: string[];
}

// Download progress
type DownloadState = 'Queued' | 'Downloading' | 'Paused' | 'Completed' | 'Failed' | 'Cancelled';

interface DownloadProgress {
  asin: string;
  title: string;
  bytes_downloaded: number;
  total_bytes: number;
  percent_complete: number;
  download_speed: number; // bytes/sec
  eta_seconds: number;
  state: DownloadState;
}
```

## Complete Examples

### Full Authentication Flow

```typescript
import { useState } from 'react';
import {
  initiateOAuth,
  completeOAuthFlow,
  getActivationBytes,
  RustBridgeError
} from '../modules/expo-rust-bridge';
import type { Account, Locale, TokenResponse } from '../modules/expo-rust-bridge';

export function useAuthentication() {
  const [oauthData, setOauthData] = useState<any>(null);

  async function startAuth(locale: Locale) {
    try {
      const flowData = initiateOAuth(locale.country_code);
      setOauthData({ flowData, locale });
      return flowData.url;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Auth start failed:', error.rustError);
      }
      throw error;
    }
  }

  async function completeAuth(callbackUrl: string): Promise<Account> {
    if (!oauthData) {
      throw new Error('No OAuth flow in progress');
    }

    try {
      const { flowData, locale } = oauthData;

      // Get tokens
      const tokens = await completeOAuthFlow(
        callbackUrl,
        locale.country_code,
        flowData.deviceSerial,
        flowData.pkceVerifier
      );

      // Create account object
      const account: Account = {
        account_id: 'user-id', // Get from API
        locale: locale,
        identity: {
          access_token: tokens.access_token,
          refresh_token: tokens.refresh_token,
          expires_at: new Date(Date.now() + tokens.expires_in * 1000).toISOString(),
          device_serial: flowData.deviceSerial,
          device_type: 'A2CZJZGLK2JJVM', // Kindle device type
        },
      };

      // Get activation bytes
      const activationBytes = await getActivationBytes(account);
      account.decrypt_key = activationBytes;

      setOauthData(null);
      return account;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Auth completion failed:', error.rustError);
      }
      throw error;
    }
  }

  return { startAuth, completeAuth };
}
```

### Library Management Component

```typescript
import { useEffect, useState } from 'react';
import {
  initializeDatabase,
  syncLibrary,
  unwrapResult,
  RustBridgeError
} from '../modules/expo-rust-bridge';
import ExpoRustBridge from '../modules/expo-rust-bridge';
import type { Book, Account, SyncStats } from '../modules/expo-rust-bridge';
import * as FileSystem from 'expo-file-system';

export function useLibrary(account: Account | null) {
  const [books, setBooks] = useState<Book[]>([]);
  const [loading, setLoading] = useState(false);
  const [syncStats, setSyncStats] = useState<SyncStats | null>(null);

  const dbPath = `${FileSystem.documentDirectory}audible.db`;

  useEffect(() => {
    // Initialize database on mount
    try {
      initializeDatabase(dbPath);
    } catch (error) {
      console.error('Database init failed:', error);
    }
  }, []);

  async function sync() {
    if (!account) {
      throw new Error('No account available');
    }

    setLoading(true);
    try {
      const stats = await syncLibrary(dbPath, account);
      setSyncStats(stats);
      await loadBooks();
      return stats;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Sync failed:', error.rustError);
      }
      throw error;
    } finally {
      setLoading(false);
    }
  }

  async function loadBooks(page: number = 0, pageSize: number = 20) {
    setLoading(true);
    try {
      const offset = page * pageSize;
      const response = ExpoRustBridge.getBooks(dbPath, offset, pageSize);
      const { books: loadedBooks } = unwrapResult(response);
      setBooks(loadedBooks);
      return loadedBooks;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Load books failed:', error.rustError);
      }
      throw error;
    } finally {
      setLoading(false);
    }
  }

  async function search(query: string) {
    setLoading(true);
    try {
      const response = ExpoRustBridge.searchBooks(dbPath, query);
      const { books: searchResults } = unwrapResult(response);
      setBooks(searchResults);
      return searchResults;
    } catch (error) {
      if (error instanceof RustBridgeError) {
        console.error('Search failed:', error.rustError);
      }
      throw error;
    } finally {
      setLoading(false);
    }
  }

  return { books, loading, syncStats, sync, loadBooks, search };
}
```

## Best Practices

1. **Always handle errors**: Wrap Rust bridge calls in try-catch blocks
2. **Use helper functions**: Prefer `initiateOAuth()` over direct module calls
3. **Type safety**: Import and use TypeScript types for better IDE support
4. **Store credentials securely**: Use `expo-secure-store` for tokens
5. **Check token expiration**: Refresh tokens before they expire
6. **Use async/await**: Most bridge functions return Promises
7. **Validate activation bytes**: Check format before attempting decryption
8. **Clean up OAuth state**: Remove temporary data after authentication completes

## Troubleshooting

### Native module not available

If you see "ExpoRustBridge native module is not available":

1. Build native libraries: `npm run build:rust:android` or `npm run build:rust:ios`
2. Rebuild the app: `npm run android` or `npm run ios`
3. Clear cache: `npx expo start -c`

### TypeScript errors

Run type checking:
```bash
npx tsc --noEmit
```

### Function not found

Ensure the function is implemented in:
- `native/rust-core/src/lib.rs` (Rust)
- `native/rust-core/src/jni_bridge.rs` (Android JNI)
- `modules/expo-rust-bridge/android/.../ExpoRustBridgeModule.kt` (Kotlin)
- `modules/expo-rust-bridge/index.ts` (TypeScript interface)

## Additional Resources

- Rust core documentation: `npm run rust:doc`
- Build scripts: `scripts/README.md`
- Implementation plan: `LIBATION_PORT_PLAN.md`
- Project overview: `CLAUDE.md`
