# LibriSync Docker Build


## Quick Start

```bash
# Build APK (outputs to build-output/debug/app-debug.apk)
./docker-build.sh build

# Development shell
./docker-build.sh dev

# Clean everything
./docker-build.sh clean
```

## What It Does

1. Installs: Node.js 20, Rust, Java 17, Android SDK 34, NDK r29
2. Builds Rust libraries for all 4 Android architectures
3. Runs Expo prebuild
4. Builds Android APK with Gradle
5. Auto-extracts APK to `build-output/debug/`

**Build Time**: 35-40 min (first), 10-15 min (subsequent with cache)

## Build Output

```
build-output/
└── debug/
    └── app-debug.apk    (171 MB, includes all Rust libs)
```

## CI/CD Integration

### GitHub Actions

```yaml
- name: Build APK
  run: ./docker-build.sh build
- uses: actions/upload-artifact@v3
  with:
    name: apk
    path: build-output/
```
