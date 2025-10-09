# FFmpeg-Kit Quick Start

## TL;DR - 3 Commands to Get Started

```bash
# 1. Install dependencies (one-time setup)
brew install autoconf automake libtool pkg-config nasm cmake yasm

# 2. Download and set up 16KB NDK (one-time setup)
# See FFMPEG_16KB_SETUP.md for the download link and setup instructions

# 3. Build FFmpeg-Kit (takes 30-60 minutes)
npm run build:ffmpeg

# 4. Integrate into project
npm run integrate:ffmpeg

# 5. Verify 16KB compatibility (after building APK)
npm run verify:16kb android/app/build/outputs/apk/debug/app-debug.apk
```

## What This Adds

- **FFmpeg-Kit library** with 16KB page size support (required for Google Play Nov 2025+)
- **AAX to M4B conversion** for Audible audiobooks
- **Audio processing** capabilities for your Rust core

## What You Need to Do Next

1. ✅ **NDK r27 already installed** - You're all set!
2. ✅ **Environment variables configured** - Already in your `.zshrc`
3. **Run `npm run build:ffmpeg`** (grab a coffee, this takes 30-60 minutes)
4. **Run `npm run integrate:ffmpeg`** to copy the .aar file
5. **Implement JNI wrappers** in your Rust code (examples in FFMPEG_16KB_SETUP.md)

## Why This Matters

- **Mandatory from Nov 1, 2025**: Google Play requires 16KB page size support
- **Old package abandoned**: `ffmpeg-kit-react-native` was removed from Maven Central
- **This is the only solution**: AliAkhgar's fork is the only maintained 16KB-compatible version

## Files Created

- ✅ `native/ffmpeg-kit-16KB/` - FFmpeg-Kit source code (cloned)
- ✅ `scripts/build-ffmpeg-kit.sh` - Build automation script
- ✅ `scripts/integrate-ffmpeg-kit.sh` - Integration script
- ✅ `scripts/check_elf_alignment.sh` - 16KB verification script
- ✅ `android/app/build.gradle` - Updated with dependencies
- ✅ `FFMPEG_16KB_SETUP.md` - Complete documentation

## Need Help?

See `FFMPEG_16KB_SETUP.md` for:
- Detailed prerequisites
- Environment setup
- Troubleshooting
- JNI wrapper examples
- Rust integration patterns

## Current Status

- [x] Removed abandoned ffmpeg-kit-react-native
- [x] Cloned 16KB-compatible repository
- [x] Created build scripts
- [x] Updated Android configuration
- [x] Added npm scripts
- [x] ✅ **NDK r27 already installed** (native 16KB support)
- [x] ✅ **Environment configured** (.zshrc updated)
- [ ] **YOU: Build FFmpeg-Kit** (`npm run build:ffmpeg`)
- [ ] **YOU: Integrate .aar file** (`npm run integrate:ffmpeg`)
- [ ] **YOU: Create JNI wrappers for audio conversion**
