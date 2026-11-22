// Calculate versionCode from unix timestamp / 10
// This ensures each build has a unique, incrementing version code
const versionCode = Math.floor(Date.now() / 10000);

// Use APP_VERSION from environment (set by CI/CD from git tag), or fallback to default
const version = process.env.APP_VERSION || "0.0.4";

export default {
  expo: {
    name: "LibriSync",
    slug: "librisync",
    // librisync://downloads opens Settings → Downloads (download notifications use it).
    scheme: "librisync",
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
            extraMavenRepos: [],
            // R8 in release builds: Google Play's "DEX code optimization" vital
            // flags apps under 25% obfuscation (deadline Feb 2027). Without it
            // v0.0.35 scored 1%; with it ~88% of classes are renamed and the DEX
            // drops to ~5.7 MB. Two things are deliberately left off:
            // - Resource shrinking: React Native looks up bundled images by name
            //   at runtime, which the shrinker can't see.
            // - The optimising profile (proguard-android-optimize.txt): tested on
            //   a Pixel 6, it broke Expo's reflection-built records
            //   (SecureStore: "cannot be cast to SecureStoreOptions").
            enableMinifyInReleaseBuilds: true,
            extraProguardRules: [
              // Our task enums' names reach JS (`task.status.name`). R8 keeps
              // name() strings intact anyway; this is a backstop, and our module
              // is a sliver of the DEX.
              "-keepclassmembers enum expo.modules.rustbridge.** { *; }",
              // Expo's ReactActivityDelegateWrapper looks these up by name
              // (isEdgeToEdgeFeatureFlagOn, enableEdgeToEdge). Neither Expo nor
              // React Native ships a rule, so under R8 edge-to-edge silently
              // stopped being applied (NoSuchMethodException at launch).
              "-keep class com.facebook.react.views.view.WindowUtilKt { *; }",
            ].join("\n")
          }
        }
      ],
      "expo-secure-store",
      "./plugins/withDebugApplicationId",
      "./plugins/withDownloadService",
      "./plugins/withFFmpegKit",
      "./plugins/withReleaseSigning",
      "./plugins/withAbiSplits",
      "./plugins/withOptionalPortraitScreen",
      "./plugins/withNdkVersion"
    ],
    extra: {
      eas: {
        projectId: "2430b726-ba32-43d1-ac5b-2a88cb22e15e"
      },
      // Enable debug screen in development mode by default
      // Override with: EXPO_PUBLIC_ENABLE_DEBUG_SCREEN=true/false
      enableDebugScreen: process.env.EXPO_PUBLIC_ENABLE_DEBUG_SCREEN === 'true' || process.env.NODE_ENV === 'development',
      // Set GITHUB_RELEASE=true during CI/APK builds to enable update checks
      isGithubRelease: process.env.GITHUB_RELEASE === 'true'
    }
  }
};
