#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== FFmpeg-Kit Integration Script ===${NC}"
echo ""

# Navigate to the project root
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FFMPEG_AAR="$PROJECT_ROOT/native/ffmpeg-kit-16KB/android/ffmpeg-kit-android-lib/build/outputs/aar/ffmpeg-kit-release.aar"
LIBS_DIR="$PROJECT_ROOT/android/app/libs"

# Check if the .aar file exists
if [ ! -f "$FFMPEG_AAR" ]; then
    echo -e "${RED}Error: ffmpeg-kit.aar not found at:${NC}"
    echo "$FFMPEG_AAR"
    echo ""
    echo -e "${YELLOW}Please build FFmpeg-Kit first by running:${NC}"
    echo "npm run build:ffmpeg"
    exit 1
fi

echo -e "${GREEN}Found ffmpeg-kit.aar${NC}"
echo ""

# Create libs directory if it doesn't exist
if [ ! -d "$LIBS_DIR" ]; then
    echo -e "${YELLOW}Creating libs directory...${NC}"
    mkdir -p "$LIBS_DIR"
fi

# Copy the .aar file
echo -e "${YELLOW}Copying ffmpeg-kit.aar to android/app/libs/${NC}"
cp "$FFMPEG_AAR" "$LIBS_DIR/ffmpeg-kit.aar"

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ Successfully copied ffmpeg-kit.aar${NC}"
else
    echo -e "${RED}✗ Failed to copy ffmpeg-kit.aar${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}=== Integration Complete ===${NC}"
echo ""
echo -e "${YELLOW}Next steps:${NC}"
echo "1. The build.gradle has been updated with the dependency"
echo "2. You can now use FFmpeg-Kit in your Rust code via JNI"
echo "3. Verify 16KB compatibility with: npm run verify:16kb"
echo ""
echo -e "${GREEN}All done!${NC}"
