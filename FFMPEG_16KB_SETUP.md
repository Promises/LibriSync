# FFmpeg-Kit 16KB Page Size Setup Guide

## Overview

This guide covers the complete setup process for integrating FFmpeg-Kit with 16KB page size support into LibriSync. Starting November 1, 2025, Google Play requires all apps targeting Android 15+ to support 16KB memory page sizes.

## What Was Done

### 1. Removed Abandoned Package
- Removed `ffmpeg-kit-react-native` (abandoned and removed from Maven Central)
- This package does not support 16KB page sizes

### 2. Cloned 16KB-Compatible Repository
- Cloned AliAkhgar's fork: https://github.com/AliAkhgar/ffmpeg-kit-16KB
- Located at: `native/ffmpeg-kit-16KB/`
- This is the only maintained fork with 16KB page size support

### 3. Created Build Scripts
- **`scripts/build-ffmpeg-kit.sh`**: Interactive build script with environment checks
- **`scripts/integrate-ffmpeg-kit.sh`**: Copies .aar file to Android project
- **`scripts/check_elf_alignment.sh`**: Verifies 16KB compatibility

### 4. Updated Android Configuration
- Modified `android/app/build.gradle` to include:
  ```gradle
  implementation(files("libs/ffmpeg-kit.aar"))
  implementation 'com.arthenica:smart-exception-java:0.1.1'
  ```

### 5. Added NPM Scripts
```json
"build:ffmpeg": "./scripts/build-ffmpeg-kit.sh",
"integrate:ffmpeg": "./scripts/integrate-ffmpeg-kit.sh",
"verify:16kb": "./scripts/check_elf_alignment.sh"
```

## Prerequisites

### Required Tools (macOS)

Install via Homebrew:
```bash
brew install autoconf automake libtool pkg-config curl git doxygen nasm cmake gcc gperf texinfo yasm bison wget gettext meson ninja ragel
```

### Android NDK with 16KB Support

**GOOD NEWS**: ✅ You already have NDK r27 (27.1.12297006) installed!

NDK r27 and higher have **native 16KB page size support** built-in. No special downloads needed.

If you don't have NDK r27:
- Open Android Studio → Settings → Android SDK → SDK Tools
- Check "Show Package Details"
- Select NDK (Side by side) version 27.1.12297006 or higher
- Click Apply

### Set Environment Variables

✅ **Already configured in your `.zshrc`!** The following variables are already set:

```bash
# Android SDK (line 114)
export ANDROID_SDK_ROOT=/Users/henningberge/Library/Android/sdk/

# Android NDK r27 with 16KB support (line 131)
export ANDROID_NDK_ROOT="$HOME/Library/Android/sdk/ndk/27.1.12297006"

# Java 17 via sdkman (automatically set by sdkman)
# JAVA_HOME is already at: $HOME/.sdkman/candidates/java/current

# PATH already includes SDK tools (line 132)
export PATH="$ANDROID_SDK_ROOT/tools:$ANDROID_SDK_ROOT/platform-tools:$ANDROID_SDK_ROOT/emulator:$PATH"
```

Reload your shell:
```bash
source ~/.zshrc  # or source ~/.bash_profile
```

## Building FFmpeg-Kit

### Option 1: Using the Build Script (Recommended)

```bash
npm run build:ffmpeg
```

This script will:
1. Check all environment variables
2. Verify required tools are installed
3. Prompt for build options
4. Build FFmpeg-Kit with 16KB support
5. Report the location of the .aar file

**Note**: This will take 30-60 minutes depending on your system.

### Option 2: Manual Build

```bash
cd native/ffmpeg-kit-16KB
./android.sh
```

The .aar file will be created at:
```
native/ffmpeg-kit-16KB/prebuilt/android-aar/ffmpeg-kit.aar
```

## Integration

### Step 1: Integrate the .aar File

```bash
npm run integrate:ffmpeg
```

This copies the .aar file to `android/app/libs/ffmpeg-kit.aar`

### Step 2: Verify 16KB Compatibility

After building your APK:
```bash
npm run verify:16kb android/app/build/outputs/apk/debug/app-debug.apk
```

You should see all libraries marked as **ALIGNED** (16KB).

## Using FFmpeg-Kit from Rust

### JNI Wrapper Example

Create a JNI wrapper in `native/rust-core/src/jni_bridge.rs`:

```rust
use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jstring;

#[no_mangle]
pub extern "C" fn Java_expo_modules_rustbridge_ExpoRustBridgeModule_nativeConvertAaxToM4b(
    env: JNIEnv,
    _class: JClass,
    input_path: JString,
    output_path: JString,
    activation_bytes: JString,
) -> jstring {
    // Get the FFmpeg-Kit session
    let ffmpeg_kit_class = env.find_class("com/arthenica/ffmpegkit/FFmpegKit").unwrap();

    // Build FFmpeg command
    let command = format!(
        "-activation_bytes {} -i {} -c copy {}",
        activation_bytes_str, input_path_str, output_path_str
    );

    // Execute FFmpeg command
    let execute_method = env.get_static_method_id(
        ffmpeg_kit_class,
        "execute",
        "(Ljava/lang/String;)Lcom/arthenica/ffmpegkit/FFmpegSession;"
    ).unwrap();

    // ... handle response
}
```

### Direct Java/Kotlin Integration

You can also call FFmpeg-Kit directly from the Kotlin Expo module:

```kotlin
import com.arthenica.ffmpegkit.FFmpegKit
import com.arthenica.ffmpegkit.ReturnCode

class ExpoRustBridgeModule : Module() {
    fun convertAaxToM4b(inputPath: String, outputPath: String, activationBytes: String) {
        val command = "-activation_bytes $activationBytes -i $inputPath -c copy $outputPath"
        val session = FFmpegKit.execute(command)

        if (ReturnCode.isSuccess(session.returnCode)) {
            // Success
        } else {
            // Handle error
            val error = session.failStackTrace
        }
    }
}
```

## Troubleshooting

### Build Fails with "NDK not found"
- Verify `ANDROID_NDK_ROOT` is set correctly
- Ensure you downloaded the 16KB-enabled NDK from Android CI
- Do NOT use the NDK from Android SDK Manager

### Missing Dependencies
```bash
brew install autoconf automake libtool pkg-config nasm cmake yasm
```

### Java Version Issues
Ensure Java 17 is being used:
```bash
java -version
# Should show version 17.x.x
```

### Build Fails with "Creating Android archive: failed"
1. Check the build logs for specific errors
2. Review `native/ffmpeg-kit-16KB/docs/troubleshooting.md`
3. Check GitHub issues: https://github.com/AliAkhgar/ffmpeg-kit-16KB/issues

### .aar File Not Found During Integration
Make sure you ran `npm run build:ffmpeg` and the build completed successfully with:
```
Creating Android archive under rebuilt: ok
```

## File Structure

```
native/
├── ffmpeg-kit-16KB/           # Cloned repository
│   ├── android.sh            # Build script
│   ├── prebuilt/             # Output directory
│   │   └── android-aar/
│   │       └── ffmpeg-kit.aar
│   └── BUILD_GUIDE.md        # Detailed build instructions
└── rust-core/                # Your Rust code

scripts/
├── build-ffmpeg-kit.sh       # Automated build script
├── integrate-ffmpeg-kit.sh   # Integration script
└── check_elf_alignment.sh    # 16KB verification script

android/
└── app/
    ├── build.gradle          # Updated with FFmpeg dependencies
    └── libs/
        └── ffmpeg-kit.aar    # Integrated .aar file
```

## Next Steps

1. **Build FFmpeg-Kit**: `npm run build:ffmpeg`
2. **Integrate**: `npm run integrate:ffmpeg`
3. **Create JNI wrappers** in `native/rust-core/src/jni_bridge.rs`
4. **Implement audio conversion** in Rust core
5. **Test with real AAX files**
6. **Verify 16KB compatibility** before submitting to Google Play

## References

- [ProAndroidDev Guide](https://proandroiddev.com/ffmpeg-kit-16-kb-page-size-in-android-d522adc5efa2)
- [Android 16KB Page Size Documentation](https://developer.android.com/guide/practices/page-sizes)
- [AliAkhgar/ffmpeg-kit-16KB](https://github.com/AliAkhgar/ffmpeg-kit-16KB)
- [FFmpeg-Kit Documentation](https://github.com/arthenica/ffmpeg-kit)

## Important Notes

- FFmpeg-Kit was officially retired in April 2025
- All binaries were removed from Maven Central
- AliAkhgar's fork is the only maintained 16KB-compatible version
- The 16KB requirement is **mandatory** starting November 1, 2025
- This setup is specific to Android; iOS uses a different approach
