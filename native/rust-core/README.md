# Rust Core Library

Shared Rust implementation of Libation functionality for both Android and iOS platforms.

## Overview

This library provides the core business logic for the RN Audible app, including:
- Audible API authentication (OAuth 2.0 with PKCE)
- Library synchronization and management
- AAX/AAXC DRM decryption
- Audio file processing
- Local database storage (SQLite)
- Download management

## Architecture

```
┌─────────────────────────────────────────────────┐
│           React Native JavaScript               │
└─────────────┬───────────────────┬───────────────┘
              │                   │
    ┌─────────▼──────────┐ ┌─────▼──────────────┐
    │   Android (JNI)    │ │   iOS (C FFI)      │
    │  Kotlin Module     │ │  Swift Module      │
    └─────────┬──────────┘ └─────┬──────────────┘
              │                   │
    ┌─────────▼───────────────────▼──────────────┐
    │         Rust Core (this crate)             │
    │  • API Client    • Crypto    • Storage     │
    │  • Auth          • Download  • Audio       │
    └────────────────────────────────────────────┘
```

## Platform Bridges

### Android (JNI Bridge)
**Location:** `src/jni_bridge.rs`

Uses JNI (Java Native Interface) to expose Rust functions to Kotlin/Java:
- Function naming: `Java_expo_modules_rustbridge_ExpoRustBridgeModule_functionName`
- Automatic memory management via JVM
- String passing via `JString` objects
- Status: **✅ Tested and working**

**Expo Module:** `modules/expo-rust-bridge/android/src/main/java/expo/modules/rustbridge/ExpoRustBridgeModule.kt`

### iOS (C FFI Bridge)
**Location:** `src/ios_bridge.rs`

Uses C FFI to expose Rust functions to Swift/Objective-C:
- Function naming: `rust_function_name`
- Manual memory management (caller must free strings)
- String passing via `*const c_char` / `*mut c_char`
- Status: **✅ Compiled and ready for integration**

**Header:** `ios_bridge.h`
**Documentation:** `SwiftIntegration.md`, `IOS_BRIDGE_IMPLEMENTATION.md`
**Expo Module:** To be created (see `SwiftIntegration.md` for examples)

## Available Functions

All functions are available on both platforms with identical JSON APIs:

### Authentication
- `generateOAuthUrl` - Generate OAuth authorization URL with PKCE
- `parseOAuthCallback` - Extract authorization code from callback URL
- `exchangeAuthCode` - Exchange auth code for access/refresh tokens
- `refreshAccessToken` - Refresh expired access token
- `getActivationBytes` - Get DRM activation bytes

### Library Management
- `initDatabase` - Initialize SQLite database
- `syncLibrary` - Sync library from Audible API
- `getBooks` - Get books with pagination
- `searchBooks` - Search books by title

### Download & Decryption
- `downloadBook` - Download audiobook file (placeholder)
- `decryptAAX` - Decrypt AAX to M4B using activation bytes

### Utilities
- `validateActivationBytes` - Validate activation bytes format
- `getSupportedLocales` - Get list of supported Audible regions

## Response Format

All functions return JSON with consistent structure:

**Success:**
```json
{
  "success": true,
  "data": { ... }
}
```

**Error:**
```json
{
  "success": false,
  "error": "Error message"
}
```

## Building

### For Android
```bash
# Build all Android architectures
./scripts/build-rust-android.sh

# Or use npm script
npm run build:rust:android
```

Outputs:
- `android/app/src/main/jniLibs/arm64-v8a/librust_core.so`
- `android/app/src/main/jniLibs/armeabi-v7a/librust_core.so`
- `android/app/src/main/jniLibs/x86/librust_core.so`
- `android/app/src/main/jniLibs/x86_64/librust_core.so`

### For iOS
```bash
# Build iOS XCFramework
./scripts/build-rust-ios.sh

# Or use npm script
npm run build:rust:ios
```

Outputs:
- `ios/Frameworks/librust_core.xcframework` (to be implemented)

### Development
```bash
# Check compilation for specific targets
cargo check --target aarch64-linux-android
cargo check --target aarch64-apple-ios
cargo check --target aarch64-apple-ios-sim

# Run tests
cargo test

# Build for development
cargo build
```

## Testing

### Unit Tests
```bash
cargo test
```

### Integration Tests
```bash
# Test with Android emulator
npm run test:android

# Test with iOS simulator (when implemented)
npm run test:ios
```

## Dependencies

Major dependencies:
- `reqwest` - HTTP client for Audible API
- `tokio` - Async runtime
- `sqlx` - SQLite database driver
- `serde` / `serde_json` - JSON serialization
- `aes` / `sha2` - Cryptography for DRM
- `jni` - JNI bindings (Android only)

## Development Notes

### Adding New Functions

1. **Implement core functionality** in appropriate module (`api/`, `crypto/`, `storage/`, etc.)

2. **Add JNI wrapper** in `src/jni_bridge.rs`:
   ```rust
   #[no_mangle]
   pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_newFunction(
       mut env: JNIEnv,
       _class: JClass,
       params: JString,
   ) -> jstring {
       // Implementation
   }
   ```

3. **Add C FFI wrapper** in `src/ios_bridge.rs`:
   ```rust
   #[no_mangle]
   pub extern "C" fn rust_new_function(params: *const c_char) -> *mut c_char {
       // Implementation
   }
   ```

4. **Update header** in `ios_bridge.h`

5. **Update Expo modules** (Kotlin and Swift)

6. **Update TypeScript** definitions in `modules/expo-rust-bridge/index.ts`

### Memory Management (iOS)

**Critical:** All strings returned by Rust must be freed:

```swift
let resultPtr = rust_some_function(args)
defer { rust_free_string(resultPtr) }
let jsonString = String(cString: resultPtr)
```

See `SwiftIntegration.md` for complete patterns.

## Documentation

- **`IMPLEMENTATION_STATUS.md`** - Detailed implementation progress
- **`SwiftIntegration.md`** - Complete Swift integration guide
- **`IOS_BRIDGE_IMPLEMENTATION.md`** - iOS bridge technical details
- **`ios_bridge.h`** - C header file with function declarations
- **`src/jni_bridge.rs`** - Android JNI implementation (with inline docs)
- **`src/ios_bridge.rs`** - iOS C FFI implementation (with inline docs)

## Project Structure

```
rust-core/
├── src/
│   ├── lib.rs                    # Main library entry point
│   ├── jni_bridge.rs            # Android JNI bridge
│   ├── ios_bridge.rs            # iOS C FFI bridge
│   ├── error.rs                 # Error types
│   ├── api/                     # Audible API client
│   │   ├── auth.rs             # OAuth authentication
│   │   ├── client.rs           # HTTP client
│   │   └── library.rs          # Library sync
│   ├── crypto/                  # Cryptography
│   │   ├── aax.rs              # AAX decryption
│   │   ├── activation.rs       # Activation bytes
│   │   └── widevine.rs         # Widevine DRM (placeholder)
│   ├── download/                # Download management
│   │   ├── manager.rs          # Download manager
│   │   ├── progress.rs         # Progress tracking
│   │   └── stream.rs           # Stream handling
│   ├── audio/                   # Audio processing
│   │   └── metadata.rs         # Metadata extraction
│   ├── storage/                 # Database
│   │   ├── database.rs         # Database connection
│   │   ├── queries.rs          # SQL queries
│   │   └── schema.rs           # Schema migrations
│   └── file/                    # File operations
│       └── naming.rs           # File naming templates
├── Cargo.toml                   # Rust dependencies
├── ios_bridge.h                 # iOS C header
├── SwiftIntegration.md          # Swift usage guide
├── IOS_BRIDGE_IMPLEMENTATION.md # iOS technical docs
├── IMPLEMENTATION_STATUS.md     # Implementation progress
└── README.md                    # This file
```

## Known Issues

1. **Download function** (`rust_download_book`) is a placeholder
   - Returns 0 bytes downloaded
   - Needs content URL from API
   - Progress tracking not implemented

2. **JNI Bridge API mismatch** (to be fixed)
   - Currently uses incorrect API signatures
   - `AudibleClient::new()` takes `Account`, not separate params
   - iOS bridge has correct implementation

3. **FFmpeg integration** not yet implemented
   - Required for AAX decryption
   - Needs platform-specific FFmpeg binaries
   - Consider using `ffmpeg-sys-next` crate

## License

[To be determined - should match main project]

## Contributing

When contributing to this crate:
1. Follow Rust naming conventions
2. Add documentation for public APIs
3. Update both JNI and C FFI bridges
4. Add unit tests for new functionality
5. Update TypeScript definitions
6. Run `cargo fmt` and `cargo clippy`

## References

- [Libation (C# reference)](https://github.com/rmcrackan/Libation)
- [Audible API Documentation](https://audible.readthedocs.io/)
- [JNI Documentation](https://docs.oracle.com/javase/8/docs/technotes/guides/jni/)
- [Rust FFI Guide](https://doc.rust-lang.org/nomicon/ffi.html)
