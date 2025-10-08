# JNI Bridge Documentation

This document provides comprehensive documentation for all JNI bridge functions that expose Rust core functionality to React Native via Kotlin.

## Architecture

```
JavaScript (React Native)
    ↓
Kotlin (ExpoRustBridgeModule)
    ↓
JNI Bridge (jni_bridge.rs)
    ↓
Rust Core (lib.rs + modules)
```

## Communication Protocol

All JNI functions use JSON for data exchange:

### Success Response
```json
{
  "success": true,
  "data": { ... }
}
```

### Error Response
```json
{
  "success": false,
  "error": "Error message"
}
```

## Function Reference

### Authentication Functions

#### 1. `generateOAuthUrl`
Generate OAuth authorization URL with PKCE for Audible login.

**JNI Signature:**
```kotlin
external fun generateOAuthUrl(paramsJson: String): String
```

**Input JSON:**
```json
{
  "locale_code": "us",
  "device_serial": "1234-5678-9012"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "authorization_url": "https://www.amazon.com/ap/signin?...",
    "pkce_verifier": "base64_encoded_verifier",
    "state": "uuid_state_value"
  }
}
```

**Supported Locales:** `us`, `uk`, `de`, `fr`, `ca`, `au`, `it`, `es`, `in`, `jp`

---

#### 2. `parseOAuthCallback`
Parse OAuth callback URL to extract authorization code.

**JNI Signature:**
```kotlin
external fun parseOAuthCallback(paramsJson: String): String
```

**Input JSON:**
```json
{
  "callback_url": "https://localhost/callback?openid.oa2.authorization_code=ABC123..."
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "authorization_code": "ABC123..."
  }
}
```

---

#### 3. `exchangeAuthCode`
Exchange authorization code for access and refresh tokens.

**JNI Signature:**
```kotlin
external fun exchangeAuthCode(paramsJson: String): String
```

**Input JSON:**
```json
{
  "locale_code": "us",
  "authorization_code": "ABC123...",
  "device_serial": "1234-5678-9012",
  "pkce_verifier": "base64_encoded_verifier"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "access_token": "...",
    "refresh_token": "...",
    "expires_in": 3600,
    "token_type": "Bearer"
  }
}
```

---

#### 4. `refreshAccessToken`
Refresh expired access token using refresh token.

**JNI Signature:**
```kotlin
external fun refreshAccessToken(paramsJson: String): String
```

**Input JSON:**
```json
{
  "locale_code": "us",
  "refresh_token": "...",
  "device_serial": "1234-5678-9012"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "access_token": "...",
    "refresh_token": "...",
    "expires_in": 3600,
    "token_type": "Bearer"
  }
}
```

---

#### 5. `getActivationBytes`
Retrieve activation bytes for DRM decryption.

**JNI Signature:**
```kotlin
external fun getActivationBytes(paramsJson: String): String
```

**Input JSON:**
```json
{
  "locale_code": "us",
  "access_token": "..."
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "activation_bytes": "1CEB00DA"
  }
}
```

---

### Library Functions

#### 6. `syncLibrary`
Synchronize library from Audible API to local database.

**JNI Signature:**
```kotlin
external fun syncLibrary(paramsJson: String): String
```

**Input JSON:**
```json
{
  "db_path": "/data/data/com.yourapp/databases/libation.db",
  "account_json": "{\"account_id\":\"user@example.com\",\"identity\":{...}}"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "total_items": 150,
    "books_added": 10,
    "books_updated": 140,
    "books_absent": 0,
    "errors": []
  }
}
```

---

#### 7. `getBooks`
Get books from database with pagination.

**JNI Signature:**
```kotlin
external fun getBooks(paramsJson: String): String
```

**Input JSON:**
```json
{
  "db_path": "/data/data/com.yourapp/databases/libation.db",
  "offset": 0,
  "limit": 50
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "books": [
      {
        "book_id": 1,
        "audible_product_id": "B012345678",
        "title": "Example Book",
        "subtitle": "A Great Story",
        "length_in_minutes": 600,
        ...
      }
    ],
    "total_count": 150
  }
}
```

---

#### 8. `searchBooks`
Search books by title.

**JNI Signature:**
```kotlin
external fun searchBooks(paramsJson: String): String
```

**Input JSON:**
```json
{
  "db_path": "/data/data/com.yourapp/databases/libation.db",
  "query": "harry potter",
  "limit": 20
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "books": [...]
  }
}
```

---

### Download Functions

#### 9. `downloadBook`
Download audiobook file from Audible CDN.

**JNI Signature:**
```kotlin
external fun downloadBook(paramsJson: String): String
```

**Input JSON:**
```json
{
  "asin": "B012345678",
  "access_token": "...",
  "locale_code": "us",
  "output_path": "/storage/emulated/0/Download/book.aax"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "bytes_downloaded": 123456789,
    "output_path": "/storage/emulated/0/Download/book.aax"
  }
}
```

**Note:** This is currently a placeholder. Full implementation requires license data and content URL retrieval.

---

### Decryption Functions

#### 10. `decryptAAX`
Decrypt AAX file to M4B format using activation bytes.

**JNI Signature:**
```kotlin
external fun decryptAAX(paramsJson: String): String
```

**Input JSON:**
```json
{
  "input_path": "/storage/emulated/0/Download/book.aax",
  "output_path": "/storage/emulated/0/Download/book.m4b",
  "activation_bytes": "1CEB00DA"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "output_path": "/storage/emulated/0/Download/book.m4b",
    "file_size": 123456789
  }
}
```

**Requirements:** FFmpeg must be installed on the device.

---

### Database Functions

#### 11. `initDatabase`
Initialize SQLite database at specified path.

**JNI Signature:**
```kotlin
external fun initDatabase(paramsJson: String): String
```

**Input JSON:**
```json
{
  "db_path": "/data/data/com.yourapp/databases/libation.db"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "initialized": true
  }
}
```

---

### Utility Functions

#### 12. `validateActivationBytes`
Validate activation bytes format.

**JNI Signature:**
```kotlin
external fun validateActivationBytes(paramsJson: String): String
```

**Input JSON:**
```json
{
  "activation_bytes": "1CEB00DA"
}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "valid": true
  }
}
```

---

#### 13. `getSupportedLocales`
Get list of supported Audible locales.

**JNI Signature:**
```kotlin
external fun getSupportedLocales(paramsJson: String): String
```

**Input JSON:**
```json
{}
```

**Output JSON:**
```json
{
  "success": true,
  "data": {
    "locales": [
      {
        "country_code": "us",
        "name": "United States",
        "domain": "audible.com",
        "with_username": true
      },
      {
        "country_code": "uk",
        "name": "United Kingdom",
        "domain": "audible.co.uk",
        "with_username": true
      },
      ...
    ]
  }
}
```

---

## Error Handling

All functions catch panics and return error responses instead of crashing. Error types include:

- **Authentication Errors**: Invalid credentials, expired tokens, OAuth failures
- **Network Errors**: Connection failures, timeouts, API errors
- **Database Errors**: SQLite errors, schema issues, constraint violations
- **File System Errors**: Permission denied, file not found, disk full
- **Validation Errors**: Invalid input, malformed JSON, incorrect data types
- **Decryption Errors**: Invalid activation bytes, corrupt files, FFmpeg failures

## Performance Considerations

### Async Operations
All I/O operations (network, database, file system) are handled asynchronously using Tokio runtime. The JNI bridge blocks on the Kotlin thread while Rust executes async operations.

### Memory Management
- JSON serialization creates temporary allocations
- Large file operations use streaming to avoid loading entire files in memory
- Database connections are pooled and reused

### Threading
- Each JNI call runs on the calling thread (usually React Native's JS thread)
- Rust async operations run on Tokio's thread pool
- Long-running operations should be called from background threads in Kotlin

## Example Kotlin Implementation

```kotlin
package expo.modules.rustbridge

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONObject

class ExpoRustBridgeModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("ExpoRustBridge")

    Function("generateOAuthUrl") { localeCode: String, deviceSerial: String ->
      val params = JSONObject()
        .put("locale_code", localeCode)
        .put("device_serial", deviceSerial)

      val result = generateOAuthUrl(params.toString())
      return@Function JSONObject(result)
    }

    Function("exchangeAuthCode") {
      localeCode: String,
      authCode: String,
      deviceSerial: String,
      pkceVerifier: String
    ->
      val params = JSONObject()
        .put("locale_code", localeCode)
        .put("authorization_code", authCode)
        .put("device_serial", deviceSerial)
        .put("pkce_verifier", pkceVerifier)

      val result = exchangeAuthCode(params.toString())
      return@Function JSONObject(result)
    }

    // ... more functions
  }

  companion object {
    init {
      System.loadLibrary("rust_core")
    }
  }

  // Native method declarations
  private external fun generateOAuthUrl(paramsJson: String): String
  private external fun exchangeAuthCode(paramsJson: String): String
  // ... more declarations
}
```

## Example React Native Usage

```typescript
import ExpoRustBridge from './modules/expo-rust-bridge';

// Generate OAuth URL
const { data } = await ExpoRustBridge.generateOAuthUrl('us', '1234-5678-9012');
console.log('Auth URL:', data.authorization_url);
console.log('PKCE Verifier:', data.pkce_verifier);

// Exchange auth code for tokens
const tokenResponse = await ExpoRustBridge.exchangeAuthCode(
  'us',
  authCode,
  deviceSerial,
  pkceVerifier
);
console.log('Access Token:', tokenResponse.data.access_token);

// Sync library
const syncResult = await ExpoRustBridge.syncLibrary(
  dbPath,
  JSON.stringify(account)
);
console.log('Books added:', syncResult.data.books_added);
console.log('Books updated:', syncResult.data.books_updated);

// Get books with pagination
const booksResult = await ExpoRustBridge.getBooks(dbPath, 0, 50);
console.log('Total books:', booksResult.data.total_count);
console.log('Books:', booksResult.data.books);
```

## Testing

### Unit Tests
Run Rust unit tests:
```bash
cargo test --lib --features jni
```

### Integration Tests
Build and run on Android:
```bash
npm run test:android
```

### Manual Testing
1. Build Rust for Android: `npm run build:rust:android`
2. Run app on device: `npm run android`
3. Test each function from React Native

## Troubleshooting

### Common Issues

1. **"UnsatisfiedLinkError: No implementation found"**
   - Ensure Rust library is built for correct architecture
   - Check that `System.loadLibrary("rust_core")` is called
   - Verify .so files are in `android/app/src/main/jniLibs/`

2. **"Invalid JSON" errors**
   - Check JSON structure matches expected format
   - Ensure all required fields are present
   - Use TypeScript types to enforce correct structure

3. **"Database locked" errors**
   - Close database connections properly
   - Don't access database from multiple threads simultaneously
   - Use connection pooling (handled by SQLx)

4. **"FFmpeg not found" errors**
   - Install FFmpeg on Android device
   - Or bundle FFmpeg libraries with app
   - Check PATH includes FFmpeg binary location

## Dependencies

### Rust Crates
- `jni` - JNI bindings
- `tokio` - Async runtime
- `serde`, `serde_json` - JSON serialization
- `lazy_static` - Static runtime initialization
- `sqlx` - Database access
- `reqwest` - HTTP client

### Build Requirements
- Rust 1.70+
- Android NDK r25c or later
- `ANDROID_NDK_HOME` environment variable set

## Next Steps

1. **Implement Kotlin Expo Module**
   - Create wrapper functions in `ExpoRustBridgeModule.kt`
   - Add external function declarations
   - Export to React Native

2. **Add TypeScript Bindings**
   - Create TypeScript interface in `index.ts`
   - Add type definitions for all functions
   - Generate documentation comments

3. **Implement Download Manager**
   - Complete `downloadBook` function
   - Add license retrieval
   - Implement progress callbacks

4. **Add Progress Callbacks**
   - For long-running operations (download, decrypt, sync)
   - Use JNI callbacks to update UI
   - Implement cancellation support

5. **Optimize Performance**
   - Profile JNI call overhead
   - Optimize JSON serialization
   - Add caching where appropriate

## References

- [JNI Specification](https://docs.oracle.com/javase/8/docs/technotes/guides/jni/)
- [Rust JNI Crate Documentation](https://docs.rs/jni/)
- [Expo Modules API](https://docs.expo.dev/modules/overview/)
- [Libation C# Source](https://github.com/rmcrackan/Libation)
