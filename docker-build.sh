#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║   LibriSync Docker Build System          ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════╝${NC}"
echo ""

# Function to display help
show_help() {
    echo -e "${GREEN}Usage:${NC} $0 [OPTION]"
    echo ""
    echo -e "${YELLOW}Options:${NC}"
    echo "  build       Build the Docker image and compile the app"
    echo "  dev         Start a development container with shell access"
    echo "  clean       Remove all build artifacts and Docker images"
    echo "  rebuild     Clean and rebuild everything from scratch"
    echo "  help        Display this help message"
    echo ""
    echo -e "${YELLOW}Examples:${NC}"
    echo "  $0 build    # Build the APK"
    echo "  $0 dev      # Start interactive dev shell"
    echo "  $0 clean    # Clean up build artifacts"
}

# Function to build the app
build_app() {
    echo -e "${GREEN}Building LibriSync in Docker...${NC}"

    # Create output directory
    mkdir -p build-output

    # Build using docker compose
    docker compose build librisync-build

    echo -e "${GREEN}Running build container...${NC}"
    docker compose run --rm librisync-build

    echo ""
    echo -e "${GREEN}✓ Build complete!${NC}"
    echo -e "${YELLOW}Build artifacts are in: ./build-output/${NC}"
    ls -lh build-output/
}

# Function to start dev container
start_dev() {
    echo -e "${GREEN}Starting development container...${NC}"
    echo -e "${YELLOW}You'll have shell access with all build tools available.${NC}"
    echo ""

    docker compose build librisync-dev
    docker compose run --rm librisync-dev
}

# Function to clean build artifacts
clean_build() {
    echo -e "${YELLOW}Cleaning build artifacts...${NC}"

    # Remove output directory
    rm -rf build-output

    # Remove Docker volumes
    docker compose down -v

    # Remove Docker images
    docker rmi librisync:latest librisync:dev 2>/dev/null || true

    echo -e "${GREEN}✓ Clean complete!${NC}"
}

# Function to rebuild everything
rebuild_all() {
    echo -e "${YELLOW}Rebuilding everything from scratch...${NC}"
    clean_build
    build_app
}

# Parse command line arguments
case "${1:-build}" in
    build)
        build_app
        ;;
    dev)
        start_dev
        ;;
    clean)
        clean_build
        ;;
    rebuild)
        rebuild_all
        ;;
    help|--help|-h)
        show_help
        ;;
    *)
        echo -e "${RED}Error: Unknown option '$1'${NC}"
        echo ""
        show_help
        exit 1
        ;;
esac
