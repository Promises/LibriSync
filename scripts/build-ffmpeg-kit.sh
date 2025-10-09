#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== FFmpeg-Kit 16KB Build Script ===${NC}"
echo ""

# Navigate to the project root
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FFMPEG_DIR="$PROJECT_ROOT/native/ffmpeg-kit-16KB"

# Check if ffmpeg-kit-16KB directory exists
if [ ! -d "$FFMPEG_DIR" ]; then
    echo -e "${RED}Error: ffmpeg-kit-16KB directory not found at $FFMPEG_DIR${NC}"
    exit 1
fi

# Check environment variables
echo -e "${YELLOW}Checking environment variables...${NC}"

if [ -z "$ANDROID_SDK_ROOT" ]; then
    echo -e "${YELLOW}ANDROID_SDK_ROOT not set. Using default: $HOME/Library/Android/sdk${NC}"
    export ANDROID_SDK_ROOT="$HOME/Library/Android/sdk"
fi

if [ ! -d "$ANDROID_SDK_ROOT" ]; then
    echo -e "${RED}Error: Android SDK not found at $ANDROID_SDK_ROOT${NC}"
    exit 1
fi

if [ -z "$ANDROID_NDK_ROOT" ]; then
    echo -e "${RED}Error: ANDROID_NDK_ROOT not set${NC}"
    echo -e "${YELLOW}Please download the 16KB-enabled NDK from:${NC}"
    echo "https://androidbuilds.storage.googleapis.com/builds/aosp_kernel-common-android15-6.6-linux-android15_r1.0/12161346/android-ndk-12161346-darwin-x86_64.tar.bz2"
    echo ""
    echo -e "${YELLOW}Then set ANDROID_NDK_ROOT to the extracted path${NC}"
    exit 1
fi

if [ ! -d "$ANDROID_NDK_ROOT" ]; then
    echo -e "${RED}Error: Android NDK not found at $ANDROID_NDK_ROOT${NC}"
    exit 1
fi

if [ -z "$JAVA_HOME" ]; then
    echo -e "${YELLOW}JAVA_HOME not set. Trying to detect...${NC}"
    # Try to find Java from Android Studio
    if [ -d "/Applications/Android Studio.app/Contents/jbr/Contents/Home" ]; then
        export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
        echo -e "${GREEN}Found Java at: $JAVA_HOME${NC}"
    else
        echo -e "${RED}Error: JAVA_HOME not set and could not auto-detect${NC}"
        exit 1
    fi
fi

export PATH="$JAVA_HOME/bin:$PATH"

echo -e "${GREEN}Environment check passed:${NC}"
echo "  ANDROID_SDK_ROOT: $ANDROID_SDK_ROOT"
echo "  ANDROID_NDK_ROOT: $ANDROID_NDK_ROOT"
echo "  JAVA_HOME: $JAVA_HOME"
echo ""

# Check Java version
JAVA_VERSION=$(java -version 2>&1 | head -n 1)
echo "  Java version: $JAVA_VERSION"
echo ""

# Check for required tools
echo -e "${YELLOW}Checking required build tools...${NC}"
MISSING_TOOLS=()

for tool in autoconf automake libtool pkg-config nasm cmake yasm; do
    if ! command -v $tool &> /dev/null; then
        MISSING_TOOLS+=($tool)
    fi
done

if [ ${#MISSING_TOOLS[@]} -gt 0 ]; then
    echo -e "${RED}Missing required tools: ${MISSING_TOOLS[@]}${NC}"
    echo -e "${YELLOW}Install with: brew install ${MISSING_TOOLS[@]}${NC}"
    exit 1
fi

echo -e "${GREEN}All required tools found${NC}"
echo ""

# Navigate to ffmpeg-kit directory
cd "$FFMPEG_DIR"

# Ask user for build options
echo -e "${YELLOW}Build options:${NC}"
echo "1. Default build (all architectures)"
echo "2. Custom build (with fontconfig)"
echo "3. Show all options"
echo ""
read -p "Select option (1-3): " choice

BUILD_CMD="./android.sh"

case $choice in
    1)
        echo -e "${GREEN}Building with default options...${NC}"
        ;;
    2)
        echo -e "${GREEN}Building with fontconfig enabled...${NC}"
        BUILD_CMD="./android.sh --enable-fontconfig"
        ;;
    3)
        ./android.sh --help
        exit 0
        ;;
    *)
        echo -e "${RED}Invalid option${NC}"
        exit 1
        ;;
esac

echo ""
echo -e "${YELLOW}Starting build... This will take 30+ minutes${NC}"
echo -e "${YELLOW}You can monitor progress in the terminal${NC}"
echo ""

# Run the build
$BUILD_CMD

# Check if build was successful
if [ $? -eq 0 ]; then
    echo ""
    echo -e "${GREEN}=== Build Successful! ===${NC}"
    echo ""
    echo -e "${GREEN}The .aar file is located at:${NC}"
    echo "$FFMPEG_DIR/prebuilt/android-aar/ffmpeg-kit.aar"
    echo ""
    echo -e "${YELLOW}Next steps:${NC}"
    echo "1. Run: npm run integrate:ffmpeg"
    echo "2. Or manually copy the .aar file to android/app/libs/"
else
    echo ""
    echo -e "${RED}=== Build Failed ===${NC}"
    echo -e "${YELLOW}Check the error messages above${NC}"
    echo -e "${YELLOW}See BUILD_GUIDE.md for troubleshooting${NC}"
    exit 1
fi
