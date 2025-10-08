#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}Building Rust native library for Android...${NC}"

# Get project root directory
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="$PROJECT_ROOT/native/rust-core"
ANDROID_LIBS_DIR="$PROJECT_ROOT/android/app/src/main/jniLibs"

# Android architectures (parallel arrays for bash 3.2 compatibility)
RUST_TARGETS=("aarch64-linux-android" "armv7-linux-androideabi" "i686-linux-android" "x86_64-linux-android")
ANDROID_ARCHS=("arm64-v8a" "armeabi-v7a" "x86" "x86_64")

# Check if NDK is available
if [ -z "$ANDROID_NDK_HOME" ] && [ -z "$ANDROID_NDK_ROOT" ]; then
    echo -e "${RED}Error: ANDROID_NDK_HOME or ANDROID_NDK_ROOT not set${NC}"
    echo "Please set one of these environment variables to your Android NDK path"
    echo "Example: export ANDROID_NDK_HOME=/Users/username/Library/Android/sdk/ndk/26.1.10909125"
    exit 1
fi

NDK_PATH="${ANDROID_NDK_HOME:-$ANDROID_NDK_ROOT}"
echo -e "${GREEN}Using NDK at: $NDK_PATH${NC}"

# Add NDK toolchain to PATH (works on both Intel and Apple Silicon Macs)
export PATH="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin:$PATH"

# Set up cross-compilation environment for all architectures
export CC_aarch64_linux_android="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/aarch64-linux-android30-clang"
export AR_aarch64_linux_android="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ar"
export CC_armv7_linux_androideabi="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/armv7a-linux-androideabi30-clang"
export AR_armv7_linux_androideabi="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ar"
export CC_i686_linux_android="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/i686-linux-android30-clang"
export AR_i686_linux_android="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ar"
export CC_x86_64_linux_android="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/x86_64-linux-android30-clang"
export AR_x86_64_linux_android="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ar"

# Install Rust targets if not already installed
echo -e "${YELLOW}Checking Rust targets...${NC}"
for target in "${RUST_TARGETS[@]}"; do
    if ! rustup target list | grep -q "$target (installed)"; then
        echo -e "${YELLOW}Installing target: $target${NC}"
        rustup target add "$target"
    fi
done

# Build for each target
cd "$RUST_DIR"

for i in "${!RUST_TARGETS[@]}"; do
    target="${RUST_TARGETS[$i]}"
    arch="${ANDROID_ARCHS[$i]}"

    echo -e "${GREEN}Building for $target ($arch)...${NC}"

    cargo build --release --target "$target"

    # Create jniLibs directory for this architecture
    mkdir -p "$ANDROID_LIBS_DIR/$arch"

    # Copy the library
    cp "target/$target/release/librust_core.so" "$ANDROID_LIBS_DIR/$arch/"

    echo -e "${GREEN}✓ Built and copied library for $arch${NC}"
done

echo -e "${GREEN}All Android architectures built successfully!${NC}"
echo -e "${YELLOW}Libraries copied to: $ANDROID_LIBS_DIR${NC}"
