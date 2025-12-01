const { withAppBuildGradle } = require('@expo/config-plugins');

/**
 * Expo config plugin to add dynamic versioning to Android build.gradle
 * - Reads version from package.json
 * - Generates versionCode from unix timestamp / 10
 */
module.exports = function withGradleVersioning(config) {
  return withAppBuildGradle(config, (config) => {
    let buildGradle = config.modResults.contents;

    // Version reading functions to insert before "android {" block
    const versionFunctions = `
/**
 * Get version from package.json
 */
def getVersionName = { ->
    try {
        def packageJson = new groovy.json.JsonSlurper().parseText(
            new File(projectRoot, 'package.json').text
        )
        return packageJson.version
    } catch (Exception e) {
        logger.warn("Could not read version: " + e.message)
        return "0.0.1"
    }
}

/**
 * Generate versionCode from unix timestamp / 10
 */
def getVersionCode = { ->
    return (int) (System.currentTimeMillis() / 10000)
}

`;

    // Insert version functions before "android {" if not already present
    if (!buildGradle.includes('def getVersionName')) {
      buildGradle = buildGradle.replace(
        /android\s*\{/,
        versionFunctions + 'android {'
      );
    }

    // Replace versionCode with dynamic function call
    buildGradle = buildGradle.replace(
      /versionCode\s+\d+/,
      'versionCode getVersionCode()'
    );

    // Replace versionName with dynamic function call
    buildGradle = buildGradle.replace(
      /versionName\s+"[^"]*"/,
      'versionName getVersionName()'
    );

    config.modResults.contents = buildGradle;
    return config;
  });
};
