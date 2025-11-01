import React, { useState, useEffect } from 'react';
import { View, Text, TouchableOpacity, ScrollView, Switch, Alert, Platform } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import { Directory, File, Paths } from 'expo-file-system';
import * as SecureStore from 'expo-secure-store';
import {
  scheduleTokenRefresh,
  scheduleLibrarySync,
  cancelTokenRefresh,
  cancelLibrarySync,
  ExpoRustBridge,
} from '../../modules/expo-rust-bridge';

const DOWNLOAD_PATH_KEY = 'download_path';
const NAMING_PATTERN_KEY = 'naming_pattern';
const AUTO_DOWNLOAD_KEY = 'auto_download';
const WIFI_ONLY_KEY = 'wifi_only';
const REMOVE_DRM_KEY = 'remove_drm';
const SYNC_FREQUENCY_KEY = 'sync_frequency';
const SYNC_WIFI_ONLY_KEY = 'sync_wifi_only';
const AUTO_TOKEN_REFRESH_KEY = 'auto_token_refresh';

type SyncFrequency = 'manual' | '1h' | '6h' | '12h' | '24h';
type NamingPattern = 'flat_file' | 'author_book_folder' | 'author_series_book';

export default function SettingsScreen() {
  const styles = useStyles(createStyles);
  const { colors } = useTheme(); // For Switch components
  const [downloadPath, setDownloadPath] = useState<string | null>(null);
  const [namingPattern, setNamingPattern] = useState<NamingPattern>('author_series_book');
  const [autoDownload, setAutoDownload] = useState(false);
  const [wifiOnly, setWifiOnly] = useState(true);
  const [removeDRM, setRemoveDRM] = useState(true);
  const [isLoading, setIsLoading] = useState(true);

  // Sync settings
  const [syncFrequency, setSyncFrequency] = useState<SyncFrequency>('manual');
  const [syncWifiOnly, setSyncWifiOnly] = useState(true);
  const [autoTokenRefresh, setAutoTokenRefresh] = useState(true);

  // Load saved settings on mount
  useEffect(() => {
    loadSettings();
  }, []);

  const loadSettings = async () => {
    try {
      const [savedPath, savedAuto, savedWifi, savedDRM, savedSyncFreq, savedSyncWifi, savedAutoRefresh] = await Promise.all([
        SecureStore.getItemAsync(DOWNLOAD_PATH_KEY),
        SecureStore.getItemAsync(AUTO_DOWNLOAD_KEY),
        SecureStore.getItemAsync(WIFI_ONLY_KEY),
        SecureStore.getItemAsync(REMOVE_DRM_KEY),
        SecureStore.getItemAsync(SYNC_FREQUENCY_KEY),
        SecureStore.getItemAsync(SYNC_WIFI_ONLY_KEY),
        SecureStore.getItemAsync(AUTO_TOKEN_REFRESH_KEY),
      ]);

      if (savedPath) setDownloadPath(savedPath);
      if (savedAuto !== null) setAutoDownload(savedAuto === 'true');
      if (savedWifi !== null) setWifiOnly(savedWifi === 'true');
      if (savedDRM !== null) setRemoveDRM(savedDRM === 'true');
      if (savedSyncFreq) setSyncFrequency(savedSyncFreq as SyncFrequency);
      if (savedSyncWifi !== null) setSyncWifiOnly(savedSyncWifi === 'true');
      if (savedAutoRefresh !== null) setAutoTokenRefresh(savedAutoRefresh === 'true');

      // Load naming pattern from native SharedPreferences
      if (Platform.OS === 'android') {
        try {
          const result = await ExpoRustBridge.getNamingPattern();
          if (result.success && result.data) {
            setNamingPattern((result.data as any).pattern as NamingPattern);
          }
        } catch (error) {
          console.error('[Settings] Failed to load naming pattern from native:', error);
        }
      }
    } catch (error) {
      console.error('[Settings] Failed to load settings:', error);
    } finally {
      setIsLoading(false);
    }
  };

  const saveSettings = async (key: string, value: string) => {
    try {
      await SecureStore.setItemAsync(key, value);
    } catch (error) {
      console.error('[Settings] Failed to save setting:', key, error);
    }
  };

  const handleChooseDirectory = async () => {
    try {
      // Use the new Directory.pickDirectoryAsync API
      const selectedDirectory = await Directory.pickDirectoryAsync(
        Platform.OS === 'android' ? undefined : Paths.document?.uri
      );

      if (selectedDirectory) {
        const selectedUri = selectedDirectory.uri;
        setDownloadPath(selectedUri);
        await saveSettings(DOWNLOAD_PATH_KEY, selectedUri);
        Alert.alert('Success', `Download directory updated successfully\n\n${(selectedDirectory as any).name || 'Selected directory'}`);
      }
    } catch (error: any) {
      console.error('[Settings] Directory picker error:', error);
      Alert.alert('Error', error.message || 'Failed to select directory');
    }
  };

  const handleAutoDownloadChange = async (value: boolean) => {
    setAutoDownload(value);
    await saveSettings(AUTO_DOWNLOAD_KEY, value.toString());
  };

  const handleWifiOnlyChange = async (value: boolean) => {
    setWifiOnly(value);
    await saveSettings(WIFI_ONLY_KEY, value.toString());
  };

  const handleRemoveDRMChange = async (value: boolean) => {
    setRemoveDRM(value);
    await saveSettings(REMOVE_DRM_KEY, value.toString());
  };

  const getSyncFrequencyLabel = (freq: SyncFrequency): string => {
    switch (freq) {
      case 'manual': return 'Manual only';
      case '1h': return 'Every hour';
      case '6h': return 'Every 6 hours';
      case '12h': return 'Every 12 hours';
      case '24h': return 'Every 24 hours';
    }
  };

  const handleSyncFrequencyPress = () => {
    const options = [
      { label: 'Manual only', value: 'manual' as SyncFrequency },
      { label: 'Every hour', value: '1h' as SyncFrequency },
      { label: 'Every 6 hours', value: '6h' as SyncFrequency },
      { label: 'Every 12 hours', value: '12h' as SyncFrequency },
      { label: 'Every 24 hours', value: '24h' as SyncFrequency },
    ];

    Alert.alert(
      'Library Sync Frequency',
      'How often should the app sync your library automatically?',
      [
        ...options.map(opt => ({
          text: opt.label,
          onPress: () => handleSyncFrequencyChange(opt.value),
        })),
        { text: 'Cancel', style: 'cancel' },
      ]
    );
  };

  const handleSyncFrequencyChange = async (value: SyncFrequency) => {
    setSyncFrequency(value);
    await saveSettings(SYNC_FREQUENCY_KEY, value);

    try {
      if (value === 'manual') {
        // Cancel library sync worker
        cancelLibrarySync();
        console.log('[Settings] Library sync worker cancelled');
      } else {
        // Schedule library sync worker with the selected interval
        const hours = parseInt(value.replace('h', ''));
        scheduleLibrarySync(hours, syncWifiOnly);
        console.log(`[Settings] Library sync scheduled: every ${hours} hours, WiFi only: ${syncWifiOnly}`);
      }
    } catch (error: any) {
      console.error('[Settings] Failed to schedule/cancel library sync:', error);
      Alert.alert('Error', error.message || 'Failed to update sync schedule');
    }
  };

  const handleSyncWifiOnlyChange = async (value: boolean) => {
    setSyncWifiOnly(value);
    await saveSettings(SYNC_WIFI_ONLY_KEY, value.toString());

    // If sync is enabled, reschedule with new WiFi setting
    if (syncFrequency !== 'manual') {
      try {
        const hours = parseInt(syncFrequency.replace('h', ''));
        scheduleLibrarySync(hours, value);
        console.log(`[Settings] Library sync rescheduled: WiFi only: ${value}`);
      } catch (error: any) {
        console.error('[Settings] Failed to reschedule library sync:', error);
        Alert.alert('Error', error.message || 'Failed to update sync WiFi setting');
      }
    }
  };

  const handleAutoTokenRefreshChange = async (value: boolean) => {
    setAutoTokenRefresh(value);
    await saveSettings(AUTO_TOKEN_REFRESH_KEY, value.toString());

    try {
      if (value) {
        // Schedule token refresh as backup (24 hours)
        // Just-in-time refresh happens before each API call, this is a safety net
        scheduleTokenRefresh(24);
        console.log('[Settings] Token refresh scheduled: every 24 hours (backup mode)');
      } else {
        // Cancel token refresh worker
        cancelTokenRefresh();
        console.log('[Settings] Token refresh worker cancelled');
      }
    } catch (error: any) {
      console.error('[Settings] Failed to schedule/cancel token refresh:', error);
      Alert.alert('Error', error.message || 'Failed to update token refresh setting');
    }
  };

  const getNamingPatternLabel = (pattern: NamingPattern): string => {
    switch (pattern) {
      case 'flat_file': return 'Flat File';
      case 'author_book_folder': return 'Author/Book Folder';
      case 'author_series_book': return 'Author/Series+Book';
    }
  };

  const getNamingPatternExample = (pattern: NamingPattern): string => {
    switch (pattern) {
      case 'flat_file': return 'All These Worlds.m4b';
      case 'author_book_folder': return 'Dennis E. Taylor/All These Worlds/All These Worlds.m4b';
      case 'author_series_book': return 'Dennis E. Taylor/Bobiverse 3 - All These Worlds/Bobiverse 3 - All These Worlds.m4b';
    }
  };

  const handleNamingPatternPress = () => {
    const options = [
      { label: 'Flat File', value: 'flat_file' as NamingPattern, example: 'All These Worlds.m4b' },
      { label: 'Author/Book Folder', value: 'author_book_folder' as NamingPattern, example: 'Author/Title/Title.m4b' },
      { label: 'Author/Series+Book', value: 'author_series_book' as NamingPattern, example: 'Author/Series X - Title/Series X - Title.m4b' },
    ];

    Alert.alert(
      'File Naming Pattern',
      'Choose how downloaded audiobooks should be organized:',
      [
        ...options.map(opt => ({
          text: `${opt.label}\n${opt.example}`,
          onPress: () => handleNamingPatternChange(opt.value),
        })),
        { text: 'Cancel', style: 'cancel' },
      ]
    );
  };

  const handleNamingPatternChange = async (value: NamingPattern) => {
    setNamingPattern(value);

    // Save to native SharedPreferences for Kotlin download code to access
    try {
      await ExpoRustBridge.setNamingPattern(value);
      console.log(`[Settings] Naming pattern changed to: ${value}`);
      Alert.alert('Success', `File naming pattern updated to: ${getNamingPatternLabel(value)}\n\n${getNamingPatternExample(value)}`);
    } catch (error: any) {
      console.error('[Settings] Failed to save naming pattern:', error);
      Alert.alert('Error', error.message || 'Failed to update naming pattern');
    }
  };

  const getDisplayPath = (path: string | null): string => {
    if (!path) return 'Not set';

    // For Android SAF URIs, extract the readable part
    if (path.includes('content://')) {
      // Extract the last part of the URI for display
      const parts = path.split('%2F');
      const lastPart = parts[parts.length - 1];
      return decodeURIComponent(lastPart || 'Selected directory');
    }

    return path;
  };

  const handleListCacheFiles = async () => {
    try {
      console.log('[Settings] Listing cache directory recursively...');
      console.log('[Settings] Cache path:', Paths.cache.uri);

      const cacheDir = new Directory(Paths.cache);

      if (!cacheDir.exists) {
        console.log('[Settings] Cache directory does not exist');
        Alert.alert('Cache Directory', 'Cache directory does not exist');
        return;
      }

      // Recursively list all files
      const fileDetails: string[] = [];
      let totalSize = 0;
      let fileCount = 0;
      let dirCount = 0;

      const listRecursive = async (dir: Directory, prefix: string = '') => {
        const items = await dir.list();

        for (const item of items) {
          const name = item.uri.split('/').filter(Boolean).pop() || 'unknown';
          const decodedName = decodeURIComponent(name);

          // Skip system/cache folders
          const skipFolders = ['WebView', 'http-cache', 'Crash Reports', 'image_cache'];
          if (skipFolders.includes(decodedName) && prefix === '') {
            continue;
          }

          // Try to treat it as a directory first
          let isDirectory = false;
          try {
            const testDir = new Directory(item.uri);
            if (testDir.exists) {
              // It's a directory
              isDirectory = true;
              dirCount++;
              const size = item.size || 0;
              const sizeStr = `${(size / 1024 / 1024).toFixed(2)} MB`;
              const logEntry = `${prefix}${decodedName}/ (${sizeStr})`;
              fileDetails.push(logEntry);
              console.log('[Settings]   ', logEntry);

              // Recurse into subdirectory
              await listRecursive(testDir, `${prefix}  `);
            }
          } catch (error) {
            // Not a directory, treat as file
            isDirectory = false;
          }

          if (!isDirectory) {
            // It's a file
            const size = item.size || 0;
            totalSize += size;
            fileCount++;
            const sizeStr = `${(size / 1024 / 1024).toFixed(2)} MB`;
            const logEntry = `${prefix}${decodedName} (${sizeStr})`;
            fileDetails.push(logEntry);
            console.log('[Settings]   ', logEntry);
          }
        }
      };

      await listRecursive(cacheDir);

      console.log('[Settings] ================');
      console.log('[Settings] Total:', fileCount, 'files,', dirCount, 'directories');
      console.log('[Settings] Total size:', (totalSize / 1024 / 1024).toFixed(2), 'MB');

      // Show summary in alert
      const summary = fileDetails.slice(0, 15).join('\n');
      const more = fileDetails.length > 15 ? `\n... and ${fileDetails.length - 15} more` : '';
      const totalSizeMB = (totalSize / 1024 / 1024).toFixed(2);
      Alert.alert(
        'Cache Directory',
        `${fileCount} files, ${dirCount} dirs\nTotal: ${totalSizeMB} MB\n\n${summary}${more}\n\nCheck console for full listing.`
      );
    } catch (error: any) {
      console.error('[Settings] Failed to list cache files:', error);
      Alert.alert('Error', error.message || 'Failed to list cache directory');
    }
  };

  const handleDeleteDatabase = () => {
    Alert.alert(
      'Delete Database',
      'This will delete all synced library data. You will need to sync again from your Audible account.\n\nAre you sure?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: async () => {
            try {
              console.log('[Settings] Deleting database files...');

              // Delete main database file
              const dbFile = new File(Paths.cache, 'audible.db');
              console.log('[Settings] Database exists:', dbFile.exists);
              if (dbFile.exists) {
                await dbFile.delete();
                console.log('[Settings] Deleted audible.db');
              }

              // Delete WAL file
              const walFile = new File(Paths.cache, 'audible.db-wal');
              console.log('[Settings] WAL exists:', walFile.exists);
              if (walFile.exists) {
                await walFile.delete();
                console.log('[Settings] Deleted audible.db-wal');
              }

              // Delete SHM file
              const shmFile = new File(Paths.cache, 'audible.db-shm');
              console.log('[Settings] SHM exists:', shmFile.exists);
              if (shmFile.exists) {
                await shmFile.delete();
                console.log('[Settings] Deleted audible.db-shm');
              }

              Alert.alert(
                'Success',
                'Database deleted successfully. Go to the Account tab to sync your library again.',
              );
              console.log('[Settings] Database deletion complete');
            } catch (error: any) {
              console.error('[Settings] Failed to delete database:', error);
              Alert.alert('Error', error.message || 'Failed to delete database');
            }
          },
        },
      ],
    );
  };

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.title}>Settings</Text>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Storage</Text>

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>Download Directory</Text>
              <Text style={styles.settingValue} numberOfLines={1}>
                {getDisplayPath(downloadPath)}
              </Text>
            </View>
            <TouchableOpacity
              style={styles.button}
              onPress={handleChooseDirectory}
              disabled={isLoading}
            >
              <Text style={styles.buttonText}>Choose</Text>
            </TouchableOpacity>
          </View>
          {downloadPath && (
            <Text style={styles.settingHint}>
              Full path: {downloadPath}
            </Text>
          )}

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>File Naming Pattern</Text>
              <Text style={styles.settingDescription}>
                How downloaded audiobooks should be organized
              </Text>
              <Text style={styles.settingHint}>
                Example: {getNamingPatternExample(namingPattern)}
              </Text>
            </View>
            <TouchableOpacity
              style={styles.button}
              onPress={handleNamingPatternPress}
              disabled={isLoading}
            >
              <Text style={styles.buttonText}>{getNamingPatternLabel(namingPattern)}</Text>
            </TouchableOpacity>
          </View>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Library Sync</Text>

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>Sync Frequency</Text>
              <Text style={styles.settingDescription}>
                How often to automatically sync your library from Audible
              </Text>
            </View>
            <TouchableOpacity
              style={styles.button}
              onPress={handleSyncFrequencyPress}
              disabled={isLoading}
            >
              <Text style={styles.buttonText}>{getSyncFrequencyLabel(syncFrequency)}</Text>
            </TouchableOpacity>
          </View>

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>Sync on Wi-Fi only</Text>
              <Text style={styles.settingDescription}>
                Only sync library when connected to Wi-Fi
              </Text>
            </View>
            <Switch
              value={syncWifiOnly}
              onValueChange={handleSyncWifiOnlyChange}
              trackColor={{ false: colors.border, true: colors.accentDim }}
              thumbColor={syncWifiOnly ? colors.accent : colors.textSecondary}
              disabled={syncFrequency === 'manual'}
            />
          </View>

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>Auto Token Refresh</Text>
              <Text style={styles.settingDescription}>
                Periodic backup check (daily). Tokens auto-refresh before each API call.
              </Text>
            </View>
            <Switch
              value={autoTokenRefresh}
              onValueChange={handleAutoTokenRefreshChange}
              trackColor={{ false: colors.border, true: colors.accentDim }}
              thumbColor={autoTokenRefresh ? colors.accent : colors.textSecondary}
            />
          </View>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Download Options</Text>

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>Auto-download new books</Text>
              <Text style={styles.settingDescription}>
                Automatically download new books when they appear in your library
              </Text>
            </View>
            <Switch
              value={autoDownload}
              onValueChange={handleAutoDownloadChange}
              trackColor={{ false: colors.border, true: colors.accentDim }}
              thumbColor={autoDownload ? colors.accent : colors.textSecondary}
            />
          </View>

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>Wi-Fi only</Text>
              <Text style={styles.settingDescription}>
                Only download over Wi-Fi connection
              </Text>
            </View>
            <Switch
              value={wifiOnly}
              onValueChange={handleWifiOnlyChange}
              trackColor={{ false: colors.border, true: colors.accentDim }}
              thumbColor={wifiOnly ? colors.accent : colors.textSecondary}
            />
          </View>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>DRM Removal</Text>

          <View style={styles.settingItem}>
            <View style={styles.settingInfo}>
              <Text style={styles.settingLabel}>Remove DRM</Text>
              <Text style={styles.settingDescription}>
                Remove DRM from downloaded audiobooks (requires activation bytes)
              </Text>
            </View>
            <Switch
              value={removeDRM}
              onValueChange={handleRemoveDRMChange}
              trackColor={{ false: colors.border, true: colors.accentDim }}
              thumbColor={removeDRM ? colors.accent : colors.textSecondary}
            />
          </View>

          {removeDRM && (
            <TouchableOpacity style={styles.card}>
              <Text style={styles.cardLabel}>Activation Bytes</Text>
              <Text style={styles.cardValue}>Not configured</Text>
              <Text style={styles.cardDescription}>
                Tap to configure activation bytes for DRM removal
              </Text>
            </TouchableOpacity>
          )}
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Database</Text>

          <TouchableOpacity
            style={styles.button}
            onPress={handleListCacheFiles}
          >
            <Text style={styles.buttonText}>
              List Cache Files
            </Text>
          </TouchableOpacity>
          <Text style={styles.settingDescription}>
            Check cache directory for leftover download/conversion files
          </Text>

          <View style={styles.spacer} />

          <TouchableOpacity
            style={[styles.button, styles.dangerButton]}
            onPress={handleDeleteDatabase}
          >
            <Text style={[styles.buttonText, styles.dangerButtonText]}>
              Delete Database
            </Text>
          </TouchableOpacity>
          <Text style={styles.dangerDescription}>
            Removes all synced library data. You'll need to sync again from Account tab.
          </Text>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>About</Text>

          <View style={styles.card}>
            <Text style={styles.cardLabel}>Version</Text>
            <Text style={styles.cardValue}>1.0.0</Text>
          </View>

          <View style={styles.card}>
            <Text style={styles.cardLabel}>Based on</Text>
            <Text style={styles.cardValue}>Libation</Text>
            <Text style={styles.cardDescription}>
              React Native port of Libation Audible client
            </Text>
          </View>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}

// Styles factory function
const createStyles = (theme: Theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  content: {
    padding: theme.spacing.lg,
    flexGrow: 1,
  },
  title: {
    ...theme.typography.title,
    marginBottom: theme.spacing.lg,
  },
  section: {
    marginBottom: theme.spacing.xl,
  },
  sectionTitle: {
    ...theme.typography.subtitle,
    marginBottom: theme.spacing.md,
  },
  settingItem: {
    flexDirection: 'row' as const,
    justifyContent: 'space-between' as const,
    alignItems: 'center' as const,
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.md,
    borderRadius: 8,
    marginBottom: theme.spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  settingInfo: {
    flex: 1,
    marginRight: theme.spacing.md,
  },
  settingLabel: {
    ...theme.typography.body,
    fontWeight: '600' as const,
    marginBottom: theme.spacing.xs,
  },
  settingValue: {
    ...theme.typography.caption,
    fontFamily: 'monospace',
  },
  settingDescription: {
    ...theme.typography.caption,
  },
  settingHint: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
    marginLeft: theme.spacing.md,
    fontFamily: 'monospace',
    fontSize: 11,
  },
  button: {
    backgroundColor: theme.colors.backgroundTertiary,
    paddingHorizontal: theme.spacing.md,
    paddingVertical: theme.spacing.sm,
    borderRadius: 6,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  buttonText: {
    ...theme.typography.body,
    fontSize: 14,
  },
  card: {
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.md,
    borderRadius: 8,
    marginBottom: theme.spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  cardLabel: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.xs,
  },
  cardValue: {
    ...theme.typography.body,
    fontWeight: '600' as const,
    marginBottom: theme.spacing.xs,
  },
  cardDescription: {
    ...theme.typography.caption,
  },
  dangerButton: {
    backgroundColor: theme.colors.backgroundSecondary,
    borderColor: theme.colors.error,
  },
  dangerButtonText: {
    color: theme.colors.error,
  },
  dangerDescription: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
    color: theme.colors.textSecondary,
    textAlign: 'center' as const,
  },
  spacer: {
    height: theme.spacing.md,
  },
});
