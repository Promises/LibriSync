# Release Build Guide

This guide explains how to build and install release versions of LibriSync.

## Prerequisites

1. **Install EAS CLI** (for production builds):
   ```bash
   npm install -g eas-cli
   ```

2. **Ensure Rust is built**:
   ```bash
   npm run build:rust:android
   ```

## Build Options

### Option 1: Quick Debug-Signed Release (Fastest)

For testing release mode with debug signing:

```bash
npm run android:release
```

This will:
- Build Rust native libraries
- Create a release APK with debug signing
- Install directly to your device/emulator

**Output**: `android/app/build/outputs/apk/release/app-release.apk`

### Option 2: EAS Local Build - Preview (Recommended for Testing)

For a properly optimized build with automatic signing:

```bash
npm run android:build:local
```

This will:
- Build Rust native libraries
- Use EAS to create a production-ready APK locally
- Handle signing automatically (generates keystore on first run)

**Output**: `*.apk` file in project root (e.g., `build-1234567890.apk`)

**First-time setup**: EAS will prompt you to:
1. Log in to your Expo account
2. Generate or provide signing credentials
3. Save credentials for future builds

### Option 3: EAS Local Build - Production

For the final production APK:

```bash
npm run android:build:production
```

This uses the production profile with additional optimizations.

## Manual Installation

After building, install the APK manually:

```bash
adb install -r path/to/your-app.apk
```

Or drag the APK file to your device and install it.

## Signing Configuration

### Automatic (Recommended)

EAS handles signing automatically:
- On first build, it generates a keystore
- Credentials are stored securely
- Future builds reuse the same credentials

### Manual (Advanced)

If you have your own keystore:

1. Place keystore in `android/app/`:
   ```bash
   cp your-keystore.keystore android/app/librisync-release-key.keystore
   ```

2. Configure EAS with your credentials:
   ```bash
   eas credentials
   ```

## Build Profiles

Defined in `eas.json`:

- **development**: Debug builds with dev client
- **preview**: Release builds for internal testing (APK)
- **production**: Final production builds (APK or AAB)

## Troubleshooting

### "eas: command not found"
Install EAS CLI:
```bash
npm install -g eas-cli
```

### Build fails with signing errors
Clear credentials and regenerate:
```bash
eas credentials
```

### APK too large
Check that Rust release build is used (not debug):
```bash
cd native/rust-core
cargo build --release --target aarch64-linux-android
```

## Google Play Upload

To create an AAB for Play Store:

1. Update `eas.json` profile to use AAB:
   ```json
   "production": {
     "android": {
       "buildType": "app-bundle"
     }
   }
   ```

2. Build:
   ```bash
   npm run android:build:production
   ```

3. Upload the `.aab` file to Google Play Console

## CI/CD

For automated builds, use EAS Build cloud:

```bash
# Build on EAS servers (not local)
eas build --platform android --profile production
```

This is free for open source projects!
