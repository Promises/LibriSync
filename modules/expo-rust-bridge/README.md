# Expo Rust Bridge

TypeScript bindings for the native Rust core library. This module provides a type-safe interface to interact with platform-specific native code (JNI for Android, FFI for iOS).

## Overview

The Expo Rust Bridge enables React Native to call Rust functions for:

- **Authentication**: OAuth flow with Audible API
- **Database**: SQLite operations for local library management
- **Library Sync**: Synchronize audiobooks from Audible
- **Download**: Download audiobooks from Audible
- **Decryption**: Remove DRM from AAX files to M4B format

## Files

- **`index.ts`**: Main module with TypeScript interface, types, and helper functions
- **`USAGE.md`**: Comprehensive usage guide with examples and API reference
- **`EXAMPLES.ts`**: Complete working examples demonstrating all features
- **`README.md`**: This file

## Quick Start

### Import the Module

```typescript
import ExpoRustBridge from '../modules/expo-rust-bridge';
import {
  initiateOAuth,
  completeOAuthFlow,
  RustBridgeError,
} from '../modules/expo-rust-bridge';
```

### Test the Bridge

```typescript
const result = ExpoRustBridge.testBridge();
console.log('Bridge status:', result);
```

### Authenticate with Audible

```typescript
// Start OAuth flow
const flowData = initiateOAuth('us');
// Open flowData.url in WebView

// Complete OAuth after callback
const tokens = await completeOAuthFlow(
  callbackUrl,
  'us',
  flowData.deviceSerial,
  flowData.pkceVerifier
);
```

## Type Definitions

All types are fully documented with JSDoc comments:

```typescript
import type {
  Account,
  Book,
  TokenResponse,
  SyncStats,
  Locale,
  DownloadProgress,
} from '../modules/expo-rust-bridge';
```

## Key Features

### 1. Full Type Safety

All functions have TypeScript types with IDE autocomplete support.

### 2. Error Handling

Custom `RustBridgeError` class for Rust-specific errors:

```typescript
try {
  const result = await someOperation();
} catch (error) {
  if (error instanceof RustBridgeError) {
    console.error('Rust error:', error.rustError);
  }
}
```

### 3. Helper Functions

High-level helper functions for common operations:

- `initiateOAuth()`: Start OAuth flow
- `completeOAuthFlow()`: Complete OAuth authentication
- `refreshToken()`: Refresh expired access token
- `getActivationBytes()`: Get DRM decryption key
- `initializeDatabase()`: Set up SQLite database
- `syncLibrary()`: Sync library from Audible

### 4. Response Wrapper

All functions return `RustResponse<T>` with success/error handling:

```typescript
interface RustResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}
```

Use `unwrapResult()` to extract data or throw error:

```typescript
const response = ExpoRustBridge.getBooks(dbPath, 0, 20);
const { books } = unwrapResult(response);
```

## Documentation

### Complete Usage Guide

See **[USAGE.md](./USAGE.md)** for:
- Installation instructions
- Authentication flow walkthrough
- Database operations
- Download & decryption
- Error handling patterns
- Complete API reference

### Working Examples

See **[EXAMPLES.ts](./EXAMPLES.ts)** for:
- Bridge testing
- OAuth authentication manager
- Library management class
- Download and decrypt workflows
- Error handling patterns
- React hooks (commented example)

## Architecture

### Native Module Flow

```
TypeScript (index.ts)
    ↓
Expo Modules Core
    ↓
Platform-specific bindings:
    Android: ExpoRustBridgeModule.kt → JNI → Rust
    iOS: UniFFI → FFI → Rust
    ↓
Rust Core (native/rust-core/)
```

### Build Process

Before using the bridge, native libraries must be built:

```bash
# Android
npm run build:rust:android

# iOS
npm run build:rust:ios

# Both platforms
npm run build:rust
```

### Platform-Specific Implementation

**Android**:
1. Rust compiled to `.so` libraries
2. JNI wrappers in `src/jni_bridge.rs`
3. Kotlin Expo module in `android/.../ExpoRustBridgeModule.kt`
4. Libraries in `android/app/src/main/jniLibs/`

**iOS** (planned):
1. Rust compiled to `.a` static libraries
2. UniFFI-generated Swift bindings
3. XCFramework in `ios/Frameworks/`

## Adding New Functions

To add a new Rust function to the bridge:

### 1. Implement in Rust

```rust
// native/rust-core/src/lib.rs
pub fn my_new_function(param: String) -> Result<String, Error> {
    // Implementation
    Ok(format!("Result: {}", param))
}
```

### 2. Add JNI Wrapper (Android)

```rust
// native/rust-core/src/jni_bridge.rs
#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_myNewFunction(
    env: JNIEnv,
    _: JClass,
    param: JString,
) -> jstring {
    // JNI implementation
}
```

### 3. Expose in Kotlin

```kotlin
// modules/expo-rust-bridge/android/.../ExpoRustBridgeModule.kt
fun myNewFunction(param: String): String {
    return myNewFunction(param)
}
```

### 4. Add TypeScript Interface

```typescript
// modules/expo-rust-bridge/index.ts
export interface ExpoRustBridgeModule {
  myNewFunction(param: string): RustResponse<{ result: string }>;
}
```

### 5. Add Helper Function (optional)

```typescript
// modules/expo-rust-bridge/index.ts
function myNewFunction(param: string): string {
  const response = NativeModule!.myNewFunction(param);
  const { result } = unwrapResult(response);
  return result;
}

export { myNewFunction };
```

## Testing

### TypeScript Type Checking

```bash
npx tsc --noEmit
```

### Bridge Functionality

```typescript
import { testBridgeConnection } from '../modules/expo-rust-bridge/EXAMPLES';

testBridgeConnection();
```

### Rust Unit Tests

```bash
npm run test:rust
```

## Troubleshooting

### Module Not Found

1. Ensure native code is built: `npm run build:rust`
2. Rebuild the app: `npm run android` or `npm run ios`
3. Clear Expo cache: `npx expo start -c`

### TypeScript Errors

Run type checking to identify issues:
```bash
npx tsc --noEmit
```

### Runtime Errors

Check native logs:
```bash
# Android
npx react-native log-android

# iOS
npx react-native log-ios
```

## Dependencies

- **expo-modules-core**: Expo's native module system (already installed)
- **Rust**: Native implementation (requires building)

No additional npm dependencies needed.

## Related Files

- Rust implementation: `/native/rust-core/src/lib.rs`
- Android JNI: `/native/rust-core/src/jni_bridge.rs`
- Kotlin module: `/modules/expo-rust-bridge/android/.../ExpoRustBridgeModule.kt`
- Build scripts: `/scripts/build-rust-*.sh`

## License

Part of the RN Audible project. See project root for license information.

## Contributing

When contributing to this module:

1. Maintain type safety with TypeScript
2. Document all public functions with JSDoc
3. Add examples to `EXAMPLES.ts`
4. Update `USAGE.md` with new features
5. Test with `npx tsc --noEmit`
6. Ensure backward compatibility

## Support

For issues, questions, or contributions:

1. Check `USAGE.md` for comprehensive documentation
2. Review `EXAMPLES.ts` for working code
3. See project `CLAUDE.md` for architecture overview
4. Consult `LIBATION_PORT_PLAN.md` for implementation roadmap
