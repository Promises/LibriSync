# RN Audible

A React Native mobile app powered by a **direct Rust port of [Libation](https://github.com/rmcrackan/Libation)** - bringing Audible library management and DRM removal to iOS and Android.

**Project Goal:** Create a 1:1 Rust library port (`libaudible`) of Libation's C# codebase, then embed it in a React Native mobile application via native bindings.

## Project Structure

```
rn-audible/
├── App.tsx                      # Main React Native app
├── modules/
│   └── expo-rust-bridge/        # TypeScript bridge to Rust native code
│       └── index.ts
├── native/
│   └── rust-core/               # Shared Rust native code
│       ├── src/
│       │   ├── lib.rs          # Main Rust module
│       │   └── rust_core.udl   # UniFFI interface definition
│       ├── Cargo.toml          # Rust dependencies
│       └── build.rs            # Build script for UniFFI
└── references/
    └── Libation/               # Original Libation source for reference
```

## Architecture

### Three-Layer Design

1. **Mobile UI Layer**: React Native + Expo (cross-platform)
2. **Native Bridge Layer**: JNI (Android) + C FFI (iOS)
3. **Core Library**: `libaudible` - **Direct Rust port of Libation**

### Libation → Rust Port

The `native/rust-core/` directory contains `libaudible` - a complete Rust translation of Libation's C# codebase:

- **Direct 1:1 port**: Maintains Libation's architecture, data models, and logic
- **Library format**: Reusable library (not a standalone app)
- **Reference-driven**: Each Rust module corresponds to a Libation C# component in `references/Libation/Source/`
- **Feature parity goal**: Implement all core Libation functionality (auth, sync, DRM, downloads)

**Ported Components:**
- `src/api/` ← `AudibleUtilities/` (Audible API, OAuth)
- `src/crypto/` ← `AaxDecrypter/` + `Widevine/` (DRM removal)
- `src/storage/` ← `DataLayer/` (SQLite database)
- `src/download/` ← `FileLiberator/` (download orchestration)
- `src/audio/` ← `FileLiberator/` (audio conversion)
- `src/file/` ← `FileManager/` (file operations)

## Getting Started

### Prerequisites

#### Required for All Platforms
- **Node.js** >= 20.16.0
- **Rust** and Cargo (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- **Expo CLI** (installed via npm)

#### Android Development
- **Android Studio** with SDK Platform 34
- **Android NDK** 26.1+ (install via Android Studio SDK Manager)
- **Java Development Kit (JDK)** 17 or higher
- Set environment variable:
  ```bash
  export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/26.1.10909125
  ```
- Install Rust Android targets:
  ```bash
  rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
  ```

#### iOS Development (macOS only)
- **Xcode** 15+ with Command Line Tools
- **CocoaPods** (`sudo gem install cocoapods`)
- Install Rust iOS targets:
  ```bash
  rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim
  ```

### Quick Start

```bash
# 1. Install dependencies
npm install

# 2. Quick development (no Rust rebuild needed)
npm start
# Then press 'a' for Android or 'i' for iOS

# 3. Full build with Rust (first time or after Rust changes)
npm run android              # Build Rust + Run on Android
npm run ios                  # Build Rust + Run on iOS
```

### Development Scripts

```bash
# Development mode (Expo Go, no native code)
npm run android:dev          # Start with Android
npm run ios:dev              # Start with iOS

# Full build with native code
npm run android              # Build Rust + Run Android
npm run ios                  # Build Rust + Run iOS

# Build Rust only
npm run build:rust           # Build for both platforms
npm run build:rust:android   # Android only
npm run build:rust:ios       # iOS only

# Testing
npm run test:rust            # Run Rust unit tests
npm run test:android         # Full test + build + run on Android
npm run test:ios             # Full test + build + run on iOS
```

## Rust Native Module

The Rust core library is located in `native/rust-core` and uses JNI (Android) and C FFI (iOS) for native bindings.

### Architecture
- **Rust Core** (`native/rust-core/`): Shared business logic
- **JNI Bridge** (`src/jni_bridge.rs`): Android-specific bindings
- **Expo Module** (`modules/expo-rust-bridge/`): React Native interface
- **Build Scripts** (`scripts/`): Cross-compilation automation

### Manual Rust Build

```bash
# Build for all Android architectures
./scripts/build-rust-android.sh

# Build for iOS
./scripts/build-rust-ios.sh

# Run Rust tests
cargo test --manifest-path native/rust-core/Cargo.toml
```

### Current Features

**Rust Core (113/113 tests passing):**
- ✅ Complete OAuth 2.0 authentication with PKCE
- ✅ Device registration and token management
- ✅ Audible API client (11 regional domains)
- ✅ SQLite database layer (11 tables)
- ✅ Library sync from Audible API
- ✅ Download manager with resume support
- ✅ AAX decryption (FFmpeg integration)
- ✅ Audio processing and metadata embedding

**React Native Integration:**
- ✅ **OAuth authentication WORKING in Android app!**
- ✅ WebView login flow with 2FA/CVF support
- ✅ Token exchange and device registration
- ✅ Account management UI
- ✅ JNI bridge (Android) - fully functional
- ✅ C FFI bridge (iOS) - compiled and ready
- ✅ Cross-compilation build scripts

## Next Steps

1. ✅ ~~OAuth authentication~~ **COMPLETE!**
2. **Extract full registration data** (device_private_key, adp_token, cookies)
3. **Library sync UI** - Display books from Audible
4. **Activation bytes** - Fix binary blob extraction
5. Download and DRM removal integration
6. Audio playback UI

## References

- [Libation (C#)](https://github.com/rmcrackan/Libation) - Original desktop application
- [UniFFI](https://mozilla.github.io/uniffi-rs/) - Rust binding generator
