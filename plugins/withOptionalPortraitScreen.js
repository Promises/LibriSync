const { withAndroidManifest } = require('expo/config-plugins');

/**
 * Expo config plugin: declare the portrait screen as *optional*.
 *
 * `orientation: "portrait"` makes Expo set `android:screenOrientation="portrait"`,
 * from which Android infers `<uses-feature android:name="android.hardware.screen.portrait"
 * android:required="true">`. Google Play then hides the app from devices without a
 * portrait screen (some Chromebooks, desktops, XR) and, since 2026-11, refuses to send
 * a release for review until it's addressed ("Required hardware feature declarations
 * are restricting your app's availability").
 *
 * Declaring the feature explicitly with `required="false"` overrides the inferred
 * requirement. Phones stay locked to portrait; devices that can't do portrait run the
 * app letterboxed instead of being unable to install it.
 */
const FEATURE = 'android.hardware.screen.portrait';

module.exports = function withOptionalPortraitScreen(config) {
  return withAndroidManifest(config, (config) => {
    const manifest = config.modResults.manifest;
    const features = (manifest['uses-feature'] = manifest['uses-feature'] || []);
    const existing = features.find((f) => f.$['android:name'] === FEATURE);
    if (existing) {
      existing.$['android:required'] = 'false';
    } else {
      features.push({ $: { 'android:name': FEATURE, 'android:required': 'false' } });
    }
    return config;
  });
};
