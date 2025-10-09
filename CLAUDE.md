# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

LibriSync is a React Native mobile app powered by **`libaudible`** - a direct Rust library port of Libation (an Audible client and DRM remover). The Rust library (`native/rust-core/`) is a complete 1:1 translation of Libation's C# codebase, maintaining the same architecture, data models, and business logic. This library is then embedded in a React Native app via JNI (Android) and C FFI (iOS) bindings.

**Project URL:** [librisync.henning.tech](https://librisync.henning.tech)

**Key Principle:** This is a **direct port**, not a rewrite. Each Rust module corresponds to a Libation C# component in `references/Libation/Source/`, and functionality is ported method-by-method to ensure feature parity.

## Key Architecture Patterns

### Three-Layer Architecture

1. **UI Layer (React Native/TypeScript)**: `App.tsx` and future UI components
2. **Bridge Layer (TypeScript)**: `modules/expo-rust-bridge/` - Currently a placeholder that will be replaced with UniFFI-generated bindings
3. **Native Core (Rust)**: `native/rust-core/` - Shared business logic compiled for both iOS and Android

### Rust-to-JavaScript Bridge

The project uses platform-specific native bindings:

**Android (JNI)**:
- **`native/rust-core/src/jni_bridge.rs`**: JNI wrapper functions that expose Rust to Kotlin
- **`modules/expo-rust-bridge/android/.../ExpoRustBridgeModule.kt`**: Kotlin Expo module that loads the native library
- Compiled Rust libraries are placed in `android/app/src/main/jniLibs/{architecture}/librust_core.so`

**iOS (C FFI)**:
- **UniFFI scaffolding**: For future iOS implementation
- Compiled libraries will be packaged as XCFramework in `ios/Frameworks/`

**Bridge Layer**:
- **`modules/expo-rust-bridge/index.ts`**: TypeScript interface with fallback for development mode
- **`modules/expo-rust-bridge/expo-module.config.json`**: Expo autolinking configuration

When adding new Rust functions:
1. Add the function to `src/lib.rs` as `pub fn`
2. Create JNI wrapper in `src/jni_bridge.rs` (Android)
3. Export through Kotlin module in `ExpoRustBridgeModule.kt`
4. Update TypeScript interface in `index.ts`

## Development Commands

### Environment Setup

```bash
# Required for Android builds
export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/29.0.14033849
```

### Quick Development (No Native Rebuild)

```bash
npm start              # Start Expo dev server (interactive menu)
npm run android:dev    # Start with Android (Expo Go mode)
npm run ios:dev        # Start with iOS (Expo Go mode)
```

### Full Build with Native Code

```bash
npm run android        # Build Rust + Android native + Run on emulator/device
npm run ios            # Build Rust + iOS native + Run on simulator/device
```

### Build Scripts

```bash
# Build Rust for specific platforms
npm run build:rust              # Build for both Android and iOS
npm run build:rust:android      # Build for Android only (all architectures)
npm run build:rust:ios          # Build for iOS only (device + simulator)

# Direct script invocation
./scripts/build-rust-android.sh # Compile to arm64-v8a, armeabi-v7a, x86, x86_64
./scripts/build-rust-ios.sh     # Create XCFramework for iOS
./scripts/build-rust.sh android # Build for specific platform
```

### Testing

```bash
npm run test:rust      # Run Rust unit tests
npm run test:android   # Full pipeline: Rust tests + build + run on Android
npm run test:ios       # Full pipeline: Rust tests + build + run on iOS
npx tsc --noEmit       # TypeScript type checking
```

### Native Directory Management

```bash
npm run prebuild:android  # Regenerate android/ folder from scratch
npm run prebuild:ios      # Regenerate ios/ folder from scratch
```

**Build Requirements**:
- Android: `ANDROID_NDK_HOME` must be set to NDK path
- iOS: Xcode command line tools must be installed
- See `scripts/README.md` for detailed setup instructions

## Code Organization

### UI Structure

```
src/
├── navigation/
│   └── AppNavigator.tsx        # Bottom tab navigation
├── screens/
│   ├── LibraryScreen.tsx       # Audiobook list with status
│   ├── AccountScreen.tsx       # Login/account management
│   └── SettingsScreen.tsx      # App settings & configuration
├── components/                  # Shared UI components (future)
└── styles/
    └── theme.ts                # Color palette, typography, spacing
```

### Styling Approach

The project uses the **Nord color theme** - a beautiful arctic-inspired color palette. All styling uses the StyleSheet API with centralized theme management.

**Theme Structure:**
- **Colors**: Nord palette (Polar Night backgrounds, Snow Storm text, Frost accents, Aurora status colors)
  - Automatically follows OS light/dark mode via `useColorScheme()`
- **Spacing**: Consistent spacing scale (xs: 4px, sm: 8px, md: 16px, lg: 24px, xl: 32px)
- **Typography**: Predefined text styles (title, subtitle, body, caption, mono)

**Recommended Pattern (Scalable):**
```typescript
import { useStyles } from '../hooks/useStyles';
import type { Theme } from '../hooks/useStyles';

function MyScreen() {
  const styles = useStyles(createStyles);
  return <View style={styles.container}>...</View>;
}

const createStyles = (theme: Theme) => ({
  container: {
    backgroundColor: theme.colors.background,
    padding: theme.spacing.lg,
  },
});
```

**Key Principles:**
- USE `useStyles` hook for automatic theme-aware styling with memoization
- NEVER use hardcoded colors, spacing, or font sizes
- Use semantic color names (e.g., `theme.colors.error`, `theme.colors.success`)
- For theme values outside styles: `const { colors } = useTheme()`
- Prefer StyleSheet over inline styles

See `src/hooks/README.md` for complete `useStyles` documentation and `THEME.md` for color palette details.

### Reference Implementation & Porting Methodology

The `references/Libation/` directory contains the original C# Libation source code, which serves as the **authoritative reference** for all implementation work.

**Direct Port Approach:**
1. **Locate the C# source** for the feature in `references/Libation/Source/`
2. **Read and understand** the complete C# implementation
3. **Translate to Rust** maintaining the same:
   - Module structure
   - Data models (classes → structs)
   - Method signatures (adjusted for Rust idioms)
   - Business logic (line-by-line where possible)
4. **Add reference comment** at the top of each Rust file:
   ```rust
   //! Direct port of Libation's XYZ functionality
   //! Reference: references/Libation/Source/ComponentName/ClassName.cs
   ```
5. **Validate behavior** matches the original implementation

**Module Mapping:**
- `src/api/` ← `AudibleUtilities/`
- `src/crypto/` ← `AaxDecrypter/` + `Widevine/`
- `src/storage/` ← `DataLayer/`
- `src/download/` ← `FileLiberator/`
- `src/audio/` ← `FileLiberator/`
- `src/file/` ← `FileManager/`

**Note**: The `references/` directory is git-ignored and should remain local only.

### Implementation Planning Documents

- **`LIBATION_PORT_PLAN.md`**: Comprehensive 15-week plan for porting Libation to Rust
  - Phase-by-phase breakdown
  - Dependency list
  - Testing strategy (unit, integration, E2E)
  - Critical challenges and solutions

- **`AGENT_IMPLEMENTATION_PLAN.md`**: Agent-assisted implementation guide for Phase 1
  - Task breakdown for general-purpose agent
  - Week-by-week execution plan
  - Success criteria and validation steps

- **`DESKTOP_DEVELOPMENT.md`**: Desktop-first development workflow
  - CLI tool for testing Rust without mobile builds
  - 10-100x faster iteration cycle
  - Watch mode, debugging, CI/CD integration

## Current Implementation Status

### ✅ Completed - Rust Core (Phase 1)
**All 113 unit tests passing (100%)**

- **Error Handling**: 58 error variants with structured context
- **HTTP Client**: Retry logic, 11 regional domains, connection pooling
- **Authentication**: OAuth 2.0 with PKCE, token exchange, device registration
- **Database Layer**: Complete SQLite schema (11 tables, 17 indexes)
- **Library Sync**: Audible API integration with **progressive page-by-page syncing**
- **Content & License APIs**: Download vouchers, DRM detection
- **Download Manager**: Resumable downloads with progress tracking
- **AAX Decryption**: FFmpeg integration with activation bytes
- **Audio Processing**: Format detection, conversion, metadata embedding
- **File Management**: Cross-platform path handling and templates

**See:** `native/rust-core/IMPLEMENTATION_STATUS.md` for detailed breakdown

### ✅ Completed - React Native App Structure
- Expo project structure with native directories
- Rust core module with JNI bindings (Android) - **TESTED AND WORKING**
- Rust core module with C FFI bindings (iOS) - **COMPILED AND READY**
- Expo native module (`ExpoRustBridgeModule.kt` for Android)
- Cross-compilation build scripts for Android (all architectures)
- Cross-compilation build scripts for iOS (device + simulator)
- Automated test and build pipeline
- TypeScript bridge with fallback for development
- Three-screen app skeleton:
  - **Library Screen**: Audiobook list with status indicators
  - **Account Screen**: Login/logout with Audible credentials
  - **Settings Screen**: Download directory, DRM options, app settings
- Bottom tab navigation with React Navigation

### ✅ Completed - iOS C FFI Bridge
- **`native/rust-core/src/ios_bridge.rs`**: Complete C FFI implementation
  - All authentication functions (OAuth, tokens, activation bytes)
  - All database functions (init, sync, get books, search)
  - Download and decryption functions
  - Utility functions (validation, locales)
  - Memory management with `rust_free_string()`
- **`native/rust-core/ios_bridge.h`**: C header file for Swift/Objective-C
- **`native/rust-core/SwiftIntegration.md`**: Complete Swift wrapper and examples
- **`native/rust-core/IOS_BRIDGE_IMPLEMENTATION.md`**: Technical documentation
- Successfully compiles for:
  - `aarch64-apple-ios` (iOS devices)
  - `aarch64-apple-ios-sim` (iOS simulator)

### ✅ OAuth Authentication - WORKING! (Oct 7, 2025)
- **OAuth 2.0 Flow**: ✅ Complete end-to-end in React Native Android app
- **WebView Integration**: ✅ Amazon login with 2FA/CVF support
- **Device Registration**: ✅ Full token exchange via `/auth/register`
- **Session Data**: ✅ Complete registration response captured
- **Test Fixture**: `native/rust-core/test_fixtures/registration_response.json`

**Key Details:**
- Device type: `A10KISP2GWF0E4` (Android)
- client_id: lowercase hex-encoded `SERIAL#DEVICETYPE`
- See `OAUTH_SUCCESS_SUMMARY.md` and `OAUTH_IMPLEMENTATION_NOTES.md`

### ✅ Paginated Library Sync - COMPLETE! (Oct 8, 2025)
- **Page-by-Page API**: ✅ `syncLibraryPage(dbPath, accountJson, page)` in Rust core
- **Progressive UI Updates**: ✅ UI updates after each page synced
- **Full Stack Implementation**: ✅ Rust → JNI/FFI → Kotlin/Swift → TypeScript → React Native
- **has_more Flag**: ✅ SyncStats includes pagination status
- **Automatic Pagination**: ✅ `syncLibrary()` loops through all pages automatically
- **Callback Support**: ✅ Optional `onPageComplete` callback for incremental UI updates

**Implementation Details:**
- `sync_library_page()` in `src/api/library.rs:638-708`
- JNI bridge: `nativeSyncLibraryPage` in `src/jni_bridge.rs:576-617`
- iOS C FFI: `rust_sync_library_page` in `src/ios_bridge.rs:524-555`
- TypeScript: `syncLibraryPage()` exported from `modules/expo-rust-bridge/index.ts`
- See `SimpleAccountScreen.tsx:301-310` for UI implementation with progress callbacks

### ✅ FFmpeg-Kit Integration - COMPLETE! (Oct 9, 2025)
- **16KB Page Size Support**: ✅ Google Play compliant (Nov 2025 requirement)
- **Complete Download Pipeline**: ✅ Download + Decrypt + Copy to user directory
- **FFmpeg-Kit**: ✅ Built from AliAkhgar/ffmpeg-kit-16KB fork (34MB .aar)
- **SAF Support**: ✅ DocumentFile APIs for content:// URIs
- **Full Stack Flow**: ✅ Rust → Kotlin → FFmpeg-Kit → SAF

**Architecture:**
1. **Rust**: Downloads encrypted AAXC file to cache, extracts decryption keys
2. **Kotlin**: Decrypts with FFmpeg-Kit (16KB page aligned)
3. **Kotlin**: Copies to user's chosen directory via SAF DocumentFile APIs
4. **Cleanup**: Deletes cache files automatically

**Implementation Details:**
- FFmpeg-Kit build: `scripts/build-ffmpeg-kit.sh`
- Integration: `scripts/integrate-ffmpeg-kit.sh`
- Verification: `scripts/check_elf_alignment.sh` (confirms 16KB alignment)
- JNI bridge: Modified `nativeDownloadBook` to return decryption keys
- Kotlin module: `ExpoRustBridgeModule.kt` orchestrates download → decrypt → copy
- Three FFmpeg-Kit functions: `convertToM4b()`, `convertAudio()`, `getAudioInfo()`

**Test Results:**
- Book: "A Mind of Her Own" (B07NP9L44Y)
- Size: 72.2 MB, Duration: 76 minutes
- Successfully saved to user's SAF directory
- Multiple downloads validated

**Build Configuration:**
- NDK r27 with native 16KB support
- .aar file: 34MB at `android/app/libs/ffmpeg-kit.aar`
- Dependencies: `com.arthenica:smart-exception-java:0.1.1`
- ELF alignment verified: 2**14 (16384 bytes = 16KB) ✅

### 🚧 Next Phase
- **Enhanced Library UI**: Cover images, sorting, filtering, search
- **Download Progress UI**: Real-time progress indicators in library list
- **iOS Expo Module**: Create Swift module using C FFI bridge
- **Activation Bytes**: Fix binary blob extraction for DRM (currently using AAXC keys)

### 📋 Next Implementation Priorities

See `LIBATION_PORT_PLAN.md` for comprehensive plan.

**Immediate priorities:**
1. ✅ ~~OAuth authentication flow~~ **COMPLETE**
2. ✅ ~~Paginated library sync~~ **COMPLETE**
3. ✅ ~~FFmpeg-Kit integration~~ **COMPLETE**
4. ✅ ~~Download and decrypt pipeline~~ **COMPLETE**
5. **Enhanced library UI** - Cover images, sorting, filtering, search
6. **Download progress UI** - Real-time progress indicators
7. **Activation bytes** - Fix binary blob extraction for AAX DRM (AAXC keys working)

## Build Architecture

### Android Build Flow
1. `build-rust-android.sh` compiles Rust to `.so` for all Android architectures
2. Libraries copied to `android/app/src/main/jniLibs/{architecture}/`
3. Gradle builds APK with embedded native libraries
4. Expo module loads library via `System.loadLibrary("rust_core")`

### iOS Build Flow (Planned)
1. `build-rust-ios.sh` compiles Rust to `.a` for iOS targets
2. Creates XCFramework combining device + simulator libraries
3. XCFramework linked in Xcode project
4. Expo module calls C FFI functions

## Important Notes

- Node.js version warnings can be ignored (works with 20.12.2+)
- The Rust module compiles to both `cdylib` and `staticlib` as configured in `Cargo.toml`
- UniFFI is kept for future cross-platform binding generation, but currently using platform-specific bindings
- Android NDK must be properly configured with `ANDROID_NDK_HOME` environment variable
- All build scripts are in `scripts/` directory - see `scripts/README.md` for details
