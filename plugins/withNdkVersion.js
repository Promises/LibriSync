const { withProjectBuildGradle } = require('expo/config-plugins');

/**
 * Expo config plugin: build every native module with the same NDK as the Rust core.
 *
 * React Native 0.81 defaults `ndkVersion` to 27.1 (r27b). Google Play flags libraries
 * built with an NDK older than r28 ("Your app may crash on 16 KB devices… compiled
 * using an older Android NDK version"), even when they are 16 KB aligned — the check
 * reads the NDK version note, not the alignment. Setting `ext.ndkVersion` before
 * `expo-root-project` is applied overrides the default (it uses `setIfNotExist`) for
 * everything compiled from source: libappmodules, the codegen libraries,
 * expo-modules-core, react-native-screens.
 *
 * Libraries that ship prebuilt in their AARs (Hermes, libreactnative, fbjni, Fresco,
 * libc++_shared) keep whatever NDK they were built with; only a React Native upgrade
 * moves those.
 *
 * Keep in step with the NDK the Dockerfile installs and `build-rust-android.sh` uses.
 */
const NDK_VERSION = '29.0.14206865';

module.exports = function withNdkVersion(config) {
  return withProjectBuildGradle(config, (config) => {
    let gradle = config.modResults.contents;
    const anchor = 'apply plugin: "expo-root-project"';
    if (!gradle.includes('ndkVersion = ')) {
      if (!gradle.includes(anchor)) {
        throw new Error(`withNdkVersion: \`${anchor}\` not found in android/build.gradle`);
      }
      gradle = gradle.replace(anchor, `ext {\n  ndkVersion = "${NDK_VERSION}"\n}\n\n${anchor}`);
    }
    config.modResults.contents = gradle;
    return config;
  });
};
