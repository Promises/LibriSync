const { withAppBuildGradle } = require('expo/config-plugins');

/**
 * Expo config plugin that lets an APK build emit one APK per ABI plus a
 * universal one, instead of a single ~200 MB APK carrying every architecture.
 *
 * Opt-in via the `abiSplits` Gradle property, so local debug builds keep their
 * single `app-debug.apk` and AAB builds (which split per ABI on Play anyway)
 * are untouched:
 *
 *   ./gradlew assembleRelease -PabiSplits=true
 *
 * Output: app-{arm64-v8a,armeabi-v7a,x86,x86_64,universal}-release.apk
 *
 * Every APK keeps the same versionCode. They are sideload artifacts for the
 * GitHub release, never uploaded to Play, so per-ABI versionCode offsets
 * would buy nothing.
 */
module.exports = function withAbiSplits(config) {
  return withAppBuildGradle(config, (config) => {
    let buildGradle = config.modResults.contents;

    if (!buildGradle.includes('splits {')) {
      buildGradle = buildGradle.replace(
        /(\n\s*)buildTypes\s*\{/,
        `$1splits {
        abi {
            enable((findProperty('abiSplits') ?: 'false').toBoolean())
            reset()
            include(*((findProperty('reactNativeArchitectures') ?: 'armeabi-v7a,arm64-v8a,x86,x86_64').split(',')))
            universalApk true
        }
    }$1buildTypes {`
      );
    }

    config.modResults.contents = buildGradle;
    return config;
  });
};
