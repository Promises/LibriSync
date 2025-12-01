// Calculate versionCode from unix timestamp / 10
// This ensures each build has a unique, incrementing version code
const versionCode = Math.floor(Date.now() / 10000);

// Use APP_VERSION from environment (set by CI/CD from git tag), or fallback to default
const version = process.env.APP_VERSION || "0.0.4";

export default {
  expo: {
    name: "LibriSync",
    slug: "librisync",
    version: version,
    orientation: "portrait",
    icon: "./assets/icon.png",
    userInterfaceStyle: "dark",
    newArchEnabled: true,
    splash: {
      image: "./assets/splash-icon.png",
      resizeMode: "contain",
      backgroundColor: "#1a1a1a"
    },
    ios: {
      supportsTablet: true,
      bundleIdentifier: "tech.henning.librisync"
    },
    android: {
      adaptiveIcon: {
        foregroundImage: "./assets/adaptive-icon.png",
        backgroundColor: "#5E81AC"
      },
      package: "tech.henning.librisync",
      versionCode: versionCode,
      edgeToEdgeEnabled: true,
      predictiveBackGestureEnabled: false,
      permissions: [
        "POST_NOTIFICATIONS",
        "FOREGROUND_SERVICE",
        "FOREGROUND_SERVICE_DATA_SYNC"
      ]
    },
    web: {
      favicon: "./assets/favicon.png"
    },
    plugins: [
      [
        "expo-build-properties",
        {
          android: {
            extraMavenRepos: []
          }
        }
      ],
      "expo-secure-store",
      "./plugins/withDownloadService",
      "./plugins/withFFmpegKit"
    ],
    extra: {
      eas: {
        projectId: "2430b726-ba32-43d1-ac5b-2a88cb22e15e"
      },
      // Enable debug screen in development mode by default
      // Override with: EXPO_PUBLIC_ENABLE_DEBUG_SCREEN=true/false
      enableDebugScreen: process.env.EXPO_PUBLIC_ENABLE_DEBUG_SCREEN === 'true' || process.env.NODE_ENV === 'development'
    }
  }
};
