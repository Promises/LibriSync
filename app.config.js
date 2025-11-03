export default {
  expo: {
    name: "LibriSync",
    slug: "librisync",
    version: "0.0.3",
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
      versionCode: 2,
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
