# iOS C FFI Bridge Implementation

## Overview

This document describes the C FFI bridge implementation for exposing Rust core functionality to iOS/Swift code in the React Native app.

## Files Created

### 1. `/native/rust-core/src/ios_bridge.rs`
Main C FFI bridge implementation with all exported functions.

**Key Features:**
- C-compatible function signatures using `extern "C"`
- Raw pointer-based string passing (`*const c_char`, `*mut c_char`)
- JSON-based data serialization for complex types
- Panic catching to prevent crashes across FFI boundary
- Tokio runtime for async operations
- Comprehensive documentation with safety notes

**Functions Implemented:**

#### Authentication Functions
- `rust_generate_oauth_url()` - Generate OAuth authorization URL with PKCE
- `rust_parse_oauth_callback()` - Parse OAuth callback URL for auth code
- `rust_exchange_auth_code()` - Exchange auth code for access/refresh tokens
- `rust_refresh_access_token()` - Refresh expired access token
- `rust_get_activation_bytes()` - Get DRM activation bytes

#### Database Functions
- `rust_init_database()` - Initialize SQLite database
- `rust_sync_library()` - Sync library from Audible API
- `rust_get_books()` - Get books with pagination
- `rust_search_books()` - Search books by title

#### Download/Decrypt Functions
- `rust_download_book()` - Download audiobook file
- `rust_decrypt_aax()` - Decrypt AAX to M4B using activation bytes

#### Utility Functions
- `rust_validate_activation_bytes()` - Validate activation bytes format
- `rust_get_supported_locales()` - Get list of supported Audible locales

#### Memory Management
- `rust_free_string()` - **CRITICAL** - Free strings returned by Rust

### 2. `/native/rust-core/ios_bridge.h`
C header file for Swift/Objective-C integration.

**Contents:**
- Function declarations for all C FFI functions
- Comprehensive documentation for each function
- Memory management warnings and usage examples
- Extern "C" wrapper for C++ compatibility

### 3. `/native/rust-core/SwiftIntegration.md`
Complete Swift integration guide with examples.

**Contents:**
- Basic pattern for calling Rust functions
- Complete Swift wrapper class (`RustBridge`)
- Strongly-typed Swift structs matching JSON responses
- Usage examples for all major features
- Error handling patterns
- Memory safety guidelines
- Expo module integration example

### 4. `/native/rust-core/IOS_BRIDGE_IMPLEMENTATION.md` (this file)
Implementation documentation and technical details.

## Architecture

### Data Flow
```
Swift/Objective-C
    ↓ (C String pointers)
C FFI Bridge (ios_bridge.rs)
    ↓ (JSON serialization)
Rust Core (lib.rs)
    ↓ (async operations)
Tokio Runtime
    ↓
Audible API / Database / Crypto
```

### Memory Management

**Critical Rule:** Every string returned by a Rust function MUST be freed by calling `rust_free_string()`.

**Swift Pattern:**
```swift
let resultPtr = rust_some_function(args)
defer { rust_free_string(resultPtr) }  // ALWAYS use defer
let jsonString = String(cString: resultPtr)
```

**Why this matters:**
- Rust allocates strings on the heap
- Ownership is transferred to Swift via raw pointer
- If not freed, memory leaks accumulate
- If freed twice, application crashes

### Error Handling

All functions return JSON with this format:

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
  "error": "Error message describing what went wrong"
}
```

This approach:
- Prevents panics from crossing FFI boundary
- Provides detailed error messages
- Allows graceful error handling in Swift
- Compatible with JavaScript error handling

### Async Operations

Rust functions that need async operations use a Tokio runtime:

```rust
lazy_static::lazy_static! {
    static ref RUNTIME: tokio::runtime::Runtime =
        tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
}

// In function:
let result = RUNTIME.block_on(async {
    // Async operations here
});
```

This allows:
- HTTP requests to Audible API
- Database operations
- File I/O operations
- All executed synchronously from Swift's perspective

## Compilation Status

✅ **Successfully compiles for:**
- `aarch64-apple-ios` (iOS devices - ARM64)
- `aarch64-apple-ios-sim` (iOS simulator - ARM64)

**Build command:**
```bash
cargo check --target aarch64-apple-ios
cargo check --target aarch64-apple-ios-sim
```

**Additional targets to support:**
- `x86_64-apple-ios` (Intel simulator) - if needed
- Build integration with `build-rust-ios.sh` script

## Differences from JNI Bridge (Android)

| Aspect | JNI Bridge | C FFI Bridge |
|--------|-----------|--------------|
| Parameter passing | `JString` objects | `*const c_char` pointers |
| Return type | `jstring` | `*mut c_char` |
| String conversion | `env.get_string()` | `CStr::from_ptr()` |
| Memory management | Automatic by JVM | Manual with `rust_free_string()` |
| Type system | Java objects | Raw pointers only |
| Function naming | `Java_package_Class_method` | `rust_method` |
| Env parameter | Required (`JNIEnv`) | Not needed |

## Integration with Expo Module

To use these functions in an Expo module, you'll need to:

1. **Copy the header file** to iOS project:
   ```bash
   cp native/rust-core/ios_bridge.h ios/Modules/ExpoRustBridge/
   ```

2. **Link the Rust library** in Xcode:
   - Add `librust_core.a` to "Link Binary With Libraries"
   - Or use XCFramework from build script

3. **Create Swift bridge** module:
   ```swift
   // RustBridge.swift
   import Foundation

   // Import C functions
   // Use SwiftIntegration.md for complete implementation
   ```

4. **Create Expo module** that wraps Swift bridge:
   ```swift
   // ExpoRustBridgeModule.swift
   import ExpoModulesCore

   public class ExpoRustBridgeModule: Module {
       public func definition() -> ModuleDefinition {
           Name("ExpoRustBridge")

           AsyncFunction("generateOAuthUrl") { ... }
           // More functions...
       }
   }
   ```

5. **Update TypeScript definitions** in `modules/expo-rust-bridge/index.ts`

## Known Issues and TODOs

### Current Implementation Notes

1. **Download function is placeholder**
   - `rust_download_book()` currently returns 0 bytes
   - Actual implementation needs content URL from API
   - Progress tracking not yet implemented

2. **Account object creation**
   - `rust_download_book()` creates minimal Account object
   - Production code should pass full Account JSON
   - Consider adding dedicated download function that takes account JSON

3. **Error types**
   - All errors returned as strings
   - Consider adding error codes for categorization
   - Would allow better error handling in Swift

### Future Enhancements

1. **Progress callbacks**
   - Download progress
   - Sync progress
   - Decryption progress
   - Requires callback FFI mechanism

2. **Streaming responses**
   - Large book lists
   - Search results
   - Would improve memory usage

3. **Cancellation support**
   - Cancel downloads
   - Cancel sync operations
   - Requires token-based cancellation

4. **Logging integration**
   - Stream Rust logs to Swift
   - Integrate with iOS logging system
   - Useful for debugging

## Testing

### Unit Tests

Rust unit tests are included in `ios_bridge.rs`:
```bash
cargo test --target aarch64-apple-ios
```

Tests cover:
- Success/error response formatting
- String conversions
- Panic catching
- Memory safety

### Integration Testing

To test the bridge:

1. **Create test Swift app** that calls each function
2. **Verify memory management** with Xcode Instruments
3. **Test error handling** with invalid inputs
4. **Benchmark performance** of FFI calls

### Example Test Swift Code

```swift
func testRustBridge() {
    // Test OAuth URL generation
    do {
        let result = try RustBridge.generateOAuthUrl(
            localeCode: "us",
            deviceSerial: "1234567890abcdef1234567890abcdef"
        )
        XCTAssertFalse(result.authorizationUrl.isEmpty)
        XCTAssertFalse(result.pkceVerifier.isEmpty)
    } catch {
        XCTFail("Failed to generate OAuth URL: \(error)")
    }

    // Test error handling with invalid locale
    XCTAssertThrowsError(
        try RustBridge.generateOAuthUrl(
            localeCode: "invalid",
            deviceSerial: "test"
        )
    )
}
```

## Performance Considerations

1. **FFI overhead**
   - Each call involves string conversion
   - JSON parsing on both sides
   - Minimal for our use case (mostly API calls)

2. **Memory allocations**
   - Each function allocates for return string
   - Freed by caller after use
   - Consider string pool for high-frequency calls

3. **Async runtime**
   - Tokio runtime created once at startup
   - Reused for all async operations
   - Efficient for I/O-bound operations

## Related Documentation

- [SwiftIntegration.md](SwiftIntegration.md) - Complete Swift usage guide
- [ios_bridge.h](ios_bridge.h) - C header file with function signatures
- [CLAUDE.md](../../CLAUDE.md) - Project architecture overview
- [LIBATION_PORT_PLAN.md](../../LIBATION_PORT_PLAN.md) - Implementation roadmap

## Code Examples

See [SwiftIntegration.md](SwiftIntegration.md) for:
- Complete RustBridge wrapper class
- OAuth flow example
- Book fetching and display
- Download and decrypt example
- Error handling patterns
- Expo module integration

## Maintenance Notes

### When Adding New Functions

1. **Add Rust function** to `ios_bridge.rs`:
   ```rust
   #[no_mangle]
   pub extern "C" fn rust_new_function(arg: *const c_char) -> *mut c_char {
       let response = catch_panic(|| {
           let arg = c_str_to_string(arg)?;
           // Implementation
           Ok(success_response(result))
       });
       string_to_c_str(response)
   }
   ```

2. **Add to header** file `ios_bridge.h`:
   ```c
   char* rust_new_function(const char* arg);
   ```

3. **Add Swift wrapper** in integration guide

4. **Update TypeScript** definitions if exposed to JS

5. **Add tests** for the new function

### When Modifying Structs

If you change any Rust structs that are serialized to JSON:

1. Update corresponding Swift structs
2. Update TypeScript type definitions
3. Run integration tests to verify compatibility
4. Update documentation with new fields

## Security Considerations

1. **Input validation**
   - All string inputs are validated
   - Null pointers are checked
   - JSON parsing is error-safe

2. **Memory safety**
   - No buffer overflows possible
   - Rust guarantees memory safety
   - Manual free required but documented

3. **Credential handling**
   - Access tokens passed as strings
   - No token caching in Rust layer
   - Swift/JS responsible for secure storage

4. **File operations**
   - Paths are validated
   - No directory traversal vulnerabilities
   - User-specified output paths allowed

## Support

For issues or questions:
1. Check [SwiftIntegration.md](SwiftIntegration.md) for usage examples
2. Review [CLAUDE.md](../../CLAUDE.md) for project architecture
3. Examine unit tests in `ios_bridge.rs`
4. Check compilation with `cargo check --target aarch64-apple-ios`
