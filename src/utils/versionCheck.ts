import Constants from 'expo-constants';
import { getSupportedAbis } from '../../modules/expo-rust-bridge';

const GITHUB_RELEASES_API = 'https://api.github.com/repos/Promises/LibriSync/releases/latest';

export interface UpdateInfo {
  currentVersion: string;
  latestVersion: string;
  downloadUrl: string;
  isUpdateAvailable: boolean;
}

function compareVersions(current: string, latest: string): boolean {
  const currentParts = current.replace(/^v/, '').split('.').map(Number);
  const latestParts = latest.replace(/^v/, '').split('.').map(Number);

  for (let i = 0; i < Math.max(currentParts.length, latestParts.length); i++) {
    const c = currentParts[i] || 0;
    const l = latestParts[i] || 0;
    if (l > c) return true;
    if (l < c) return false;
  }
  return false;
}

interface ReleaseAsset {
  name?: string;
  browser_download_url?: string;
}

/**
 * Releases carry `librisync-vX-<abi>.apk` for each ABI plus
 * `librisync-universal-vX.apk` (v0.0.31 named it `librisync-vX-universal.apk`).
 * Prefer the device's own ABI — a fraction of the universal APK's size — then
 * universal, then any APK (older releases shipped a single unsuffixed one).
 */
function pickApkAsset(assets: ReleaseAsset[]): ReleaseAsset | undefined {
  const apks = assets.filter(a => a.name?.endsWith('.apk'));

  let abis: string[] = [];
  try {
    abis = getSupportedAbis();
  } catch {
    // Older native module or non-Android platform: fall through to universal.
  }

  for (const abi of abis) {
    const match = apks.find(a => a.name?.endsWith(`-${abi}.apk`));
    if (match) return match;
  }

  return apks.find(a => a.name?.includes('-universal')) ?? apks[0];
}

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  try {
    const response = await fetch(GITHUB_RELEASES_API, {
      headers: { 'Accept': 'application/vnd.github.v3+json' },
    });

    if (!response.ok) return null;

    const release = await response.json();
    const latestVersion = release.tag_name as string;
    const currentVersion = Constants.expoConfig?.version || '0.0.0';

    const apkAsset = pickApkAsset(release.assets ?? []);
    const downloadUrl = apkAsset?.browser_download_url || release.html_url;

    return {
      currentVersion,
      latestVersion: latestVersion.replace(/^v/, ''),
      downloadUrl,
      isUpdateAvailable: compareVersions(currentVersion, latestVersion),
    };
  } catch {
    return null;
  }
}

export function isGithubReleaseBuild(): boolean {
  return Constants.expoConfig?.extra?.isGithubRelease === true;
}
