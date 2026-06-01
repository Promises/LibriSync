const fs = require('fs');
const path = require('path');
const { withAppBuildGradle, withDangerousMod } = require('expo/config-plugins');

/**
 * Expo config plugin that gives debug builds their own application id and
 * launcher label, so a debug/test build can be installed side-by-side with a
 * release build of LibriSync (separate app data, separate SAF grants, separate
 * login). Release builds are untouched.
 *
 *   release: tech.henning.librisync        ("LibriSync")
 *   debug:   tech.henning.librisync.debug   ("LibriSync Debug")
 */
module.exports = function withDebugApplicationId(config) {
  config = withAppBuildGradle(config, (config) => {
    let buildGradle = config.modResults.contents;

    if (!buildGradle.includes('applicationIdSuffix ".debug"')) {
      buildGradle = buildGradle.replace(
        /debug\s*\{\s*signingConfig\s+signingConfigs\.debug/,
        `debug {
            signingConfig signingConfigs.debug
            applicationIdSuffix ".debug"
            versionNameSuffix "-debug"`
      );
    }

    config.modResults.contents = buildGradle;
    return config;
  });

  // Override the launcher label for debug builds only.
  config = withDangerousMod(config, [
    'android',
    (config) => {
      const debugValuesDir = path.join(
        config.modRequest.platformProjectRoot,
        'app',
        'src',
        'debug',
        'res',
        'values'
      );
      fs.mkdirSync(debugValuesDir, { recursive: true });
      fs.writeFileSync(
        path.join(debugValuesDir, 'strings.xml'),
        `<?xml version="1.0" encoding="utf-8"?>
<resources>
  <string name="app_name">LibriSync Debug</string>
</resources>
`
      );
      return config;
    },
  ]);

  return config;
};
