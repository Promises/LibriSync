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
# Google Play requires 16 KB page alignment on the 64-bit ABIs (Nov 2025). Newer
# NDKs default to this, older ones link at 4 KB — so ask for it explicitly rather
# than depending on which NDK happens to be installed.
NEEDS_16KB=("yes" "no" "no" "yes")
LINK_16KB="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"

# Check if NDK is available
if [ -z "$ANDROID_NDK_HOME" ] && [ -z "$ANDROID_NDK_ROOT" ]; then
    echo -e "${RED}Error: ANDROID_NDK_HOME or ANDROID_NDK_ROOT not set${NC}"
    echo "Please set one of these environment variables to your Android NDK path"
    echo "Example: export ANDROID_NDK_HOME=/Users/username/Library/Android/sdk/ndk/26.1.10909125"
    exit 1
fi

NDK_PATH="${ANDROID_NDK_HOME:-$ANDROID_NDK_ROOT}"
echo -e "${GREEN}Using NDK at: $NDK_PATH${NC}"

# Detect the host toolchain directory used by the Android NDK.
HOST_TAG="linux-x86_64"
case "$(uname -s)" in
    Darwin)
        HOST_TAG="darwin-x86_64"
        ;;
    Linux)
        HOST_TAG="linux-x86_64"
        ;;
    *)
        echo -e "${RED}Unsupported host OS: $(uname -s)${NC}"
        exit 1
        ;;
esac

TOOLCHAIN_BIN="$NDK_PATH/toolchains/llvm/prebuilt/$HOST_TAG/bin"
if [ ! -d "$TOOLCHAIN_BIN" ]; then
    echo -e "${RED}NDK toolchain not found: $TOOLCHAIN_BIN${NC}"
    exit 1
fi

echo -e "${GREEN}Using NDK host toolchain: $HOST_TAG${NC}"
export PATH="$TOOLCHAIN_BIN:$PATH"

# Set up cross-compilation environment for all architectures
export CC_aarch64_linux_android="$TOOLCHAIN_BIN/aarch64-linux-android30-clang"
export AR_aarch64_linux_android="$TOOLCHAIN_BIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOLCHAIN_BIN/aarch64-linux-android30-clang"

export CC_armv7_linux_androideabi="$TOOLCHAIN_BIN/armv7a-linux-androideabi30-clang"
export AR_armv7_linux_androideabi="$TOOLCHAIN_BIN/llvm-ar"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$TOOLCHAIN_BIN/armv7a-linux-androideabi30-clang"

export CC_i686_linux_android="$TOOLCHAIN_BIN/i686-linux-android30-clang"
export AR_i686_linux_android="$TOOLCHAIN_BIN/llvm-ar"
export CARGO_TARGET_I686_LINUX_ANDROID_LINKER="$TOOLCHAIN_BIN/i686-linux-android30-clang"

export CC_x86_64_linux_android="$TOOLCHAIN_BIN/x86_64-linux-android30-clang"
export AR_x86_64_linux_android="$TOOLCHAIN_BIN/llvm-ar"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$TOOLCHAIN_BIN/x86_64-linux-android30-clang"

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

    if [ "${NEEDS_16KB[$i]}" = "yes" ]; then
        RUSTFLAGS="$LINK_16KB" cargo build --release --target "$target"
    else
        cargo build --release --target "$target"
    fi

    # Create jniLibs directory for this architecture
    mkdir -p "$ANDROID_LIBS_DIR/$arch"

    # Copy the library
    cp "target/$target/release/librust_core.so" "$ANDROID_LIBS_DIR/$arch/"

    echo -e "${GREEN}✓ Built and copied library for $arch${NC}"
done

echo -e "${GREEN}All Android architectures built successfully!${NC}"
echo -e "${YELLOW}Libraries copied to: $ANDROID_LIBS_DIR${NC}"

# Fail loudly rather than shipping a Play-noncompliant library. Only the 64-bit
# ABIs are subject to the 16 KB requirement.
echo -e "${YELLOW}Verifying 16 KB page alignment...${NC}"
for i in "${!ANDROID_ARCHS[@]}"; do
    [ "${NEEDS_16KB[$i]}" = "yes" ] || continue
    lib="$ANDROID_LIBS_DIR/${ANDROID_ARCHS[$i]}/librust_core.so"
    if ! "$TOOLCHAIN_BIN/llvm-objdump" -p "$lib" | awk '/LOAD/ { if ($NF == "2**14" || $NF == "2**16") ok = 1 } END { exit !ok }'; then
        echo -e "${RED}$lib is not 16 KB aligned — Google Play will reject this build.${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ ${ANDROID_ARCHS[$i]} is 16 KB aligned${NC}"
done
