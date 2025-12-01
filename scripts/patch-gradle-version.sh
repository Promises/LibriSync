#!/bin/bash
# Patch android/app/build.gradle to add dynamic version reading
# This script runs after expo prebuild in Docker builds

set -e

GRADLE_FILE="android/app/build.gradle"

if [ ! -f "$GRADLE_FILE" ]; then
    echo "Error: $GRADLE_FILE not found"
    exit 1
fi

echo "Patching $GRADLE_FILE with dynamic version code and name..."

# Use perl to do in-place editing (more reliable than awk/sed combos)
perl -i -pe '
BEGIN {
    $version_funcs = q{
/**
 * Get version from app.config.js (preferred) or package.json (fallback)
 */
def getVersionName = { ->
    try {
        def packageJson = new groovy.json.JsonSlurper().parseText(
            new File(projectRoot, "package.json").text
        )
        return packageJson.version
    } catch (Exception e) {
        logger.warn("Could not read version: " + e.message)
        return "0.0.3"
    }
}

/**
 * Generate versionCode from unix timestamp / 10
 */
def getVersionCode = { ->
    return (int) (System.currentTimeMillis() / 10000)
}

};
    $printed_funcs = 0;
}

# Insert version functions before "android {" block
if (/^android \{/ && !$printed_funcs) {
    print $version_funcs;
    $printed_funcs = 1;
}

# Replace versionCode line
s/(versionCode\s+)\d+/${1}getVersionCode()/;

# Replace versionName line
s/(versionName\s+)"[^"]*"/${1}getVersionName()/;

' "$GRADLE_FILE"

echo "✓ Patched $GRADLE_FILE successfully"
