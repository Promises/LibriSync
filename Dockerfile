# Multi-stage Dockerfile for LibriSync React Native + Rust build
# Force AMD64 platform for better compatibility with Android NDK
FROM --platform=linux/amd64 ubuntu:22.04 as builder

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# Install system dependencies
RUN apt-get update && apt-get install -y \
    curl \
    wget \
    git \
    unzip \
    build-essential \
    pkg-config \
    libssl-dev \
    python3 \
    python3-pip \
    openjdk-17-jdk \
    && rm -rf /var/lib/apt/lists/*

# Set Java environment variables
ENV JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
ENV PATH=$PATH:$JAVA_HOME/bin

# Install Node.js 20.x
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable \
    && chmod -R a+w /usr/local/rustup /usr/local/cargo

# Install Android SDK command line tools
ENV ANDROID_SDK_ROOT=/opt/android-sdk
ENV ANDROID_HOME=/opt/android-sdk
ENV PATH=$PATH:$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:$ANDROID_SDK_ROOT/platform-tools

RUN mkdir -p $ANDROID_SDK_ROOT/cmdline-tools \
    && cd $ANDROID_SDK_ROOT/cmdline-tools \
    && wget https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip \
    && unzip commandlinetools-linux-*.zip \
    && rm commandlinetools-linux-*.zip \
    && mv cmdline-tools latest

# Accept Android SDK licenses and install required packages
RUN yes | $ANDROID_SDK_ROOT/cmdline-tools/latest/bin/sdkmanager --licenses --sdk_root=$ANDROID_SDK_ROOT || true
RUN $ANDROID_SDK_ROOT/cmdline-tools/latest/bin/sdkmanager --sdk_root=$ANDROID_SDK_ROOT --verbose \
    "platform-tools" \
    "platforms;android-34" \
    "build-tools;34.0.0" \
    "ndk;29.0.14033849" \
    "cmake;3.22.1" || (cat /root/.android/logs/*.log && exit 1)

# Set NDK environment variable
ENV ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/29.0.14033849
ENV ANDROID_NDK_ROOT=$ANDROID_SDK_ROOT/ndk/29.0.14033849
ENV PATH=$PATH:$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin

# Install Rust Android targets
RUN rustup target add \
    aarch64-linux-android \
    armv7-linux-androideabi \
    i686-linux-android \
    x86_64-linux-android

# Create working directory
WORKDIR /app

# Copy package files first for better caching
COPY package*.json ./

# Install Node.js dependencies
RUN npm ci

# Copy the rest of the application
COPY . .

# Save FFmpeg-Kit AAR before Expo prebuild wipes it
RUN if [ -f android/app/libs/ffmpeg-kit.aar ]; then \
        cp android/app/libs/ffmpeg-kit.aar /tmp/ffmpeg-kit.aar && \
        echo "Saved ffmpeg-kit.aar ($(du -h /tmp/ffmpeg-kit.aar | cut -f1))"; \
    fi

# Generate native Android project with Expo prebuild
# This creates the android directory structure (will wipe existing android/)
RUN npx expo prebuild --platform android --clean

# Restore FFmpeg-Kit AAR after prebuild
RUN mkdir -p android/app/libs && \
    if [ -f /tmp/ffmpeg-kit.aar ]; then \
        cp /tmp/ffmpeg-kit.aar android/app/libs/ && \
        echo "Restored ffmpeg-kit.aar to android/app/libs/"; \
    else \
        echo "Warning: ffmpeg-kit.aar not found, build may fail"; \
    fi

# Build Rust libraries for Android AFTER prebuild
# This ensures .so files are copied to the generated android directory
RUN chmod +x ./scripts/build-rust-android-docker.sh \
    && ./scripts/build-rust-android-docker.sh

# Build Android APK
RUN cd android && ./gradlew assembleDebug

# Final stage - lighter image with just the build artifacts
FROM --platform=linux/amd64 ubuntu:22.04

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy build artifacts from builder
COPY --from=builder /app/android/app/build/outputs/apk /app/build/apk
COPY --from=builder /app/package.json /app/

# Create a volume for output
VOLUME /output

# Command to copy APK to output directory
CMD ["sh", "-c", "cp -r /app/build/apk/* /output/ && echo 'Build artifacts copied to /output/'"]
