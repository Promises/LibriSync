import React, { useState, useEffect } from 'react';
import { View, Text, StyleSheet, TouchableOpacity, ScrollView, Switch, Alert, Platform } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { colors, spacing, typography } from '../styles/theme';
import { Directory, File, Paths } from 'expo-file-system';
import * as SecureStore from 'expo-secure-store';

const DOWNLOAD_PATH_KEY = 'download_path';
const AUTO_DOWNLOAD_KEY = 'auto_download';
const WIFI_ONLY_KEY = 'wifi_only';
const REMOVE_DRM_KEY = 'remove_drm';

export default function SettingsScreen() {
  const [downloadPath, setDownloadPath] = useState<string | null>(null);
  const [autoDownload, setAutoDownload] = useState(false);
  const [wifiOnly, setWifiOnly] = useState(true);
  const [removeDRM, setRemoveDRM] = useState(true);
  const [isLoading, setIsLoading] = useState(true);

  // Load saved settings on mount
  useEffect(() => {
    loadSettings();
  }, []);

  const loadSettings = async () => {
    try {
      const [savedPath, savedAuto, savedWifi, savedDRM] = await Promise.all([
        SecureStore.getItemAsync(DOWNLOAD_PATH_KEY),
        SecureStore.getItemAsync(AUTO_DOWNLOAD_KEY),
        SecureStore.getItemAsync(WIFI_ONLY_KEY),
        SecureStore.getItemAsync(REMOVE_DRM_KEY),
      ]);

      if (savedPath) setDownloadPath(savedPath);
      if (savedAuto !== null) setAutoDownload(savedAuto === 'true');
      if (savedWifi !== null) setWifiOnly(savedWifi === 'true');
      if (savedDRM !== null) setRemoveDRM(savedDRM === 'true');
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

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: colors.background,
  },
  content: {
    padding: spacing.lg,
    flexGrow: 1,
  },
  title: {
    ...typography.title,
    marginBottom: spacing.lg,
  },
  section: {
    marginBottom: spacing.xl,
  },
  sectionTitle: {
    ...typography.subtitle,
    marginBottom: spacing.md,
  },
  settingItem: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    backgroundColor: colors.backgroundSecondary,
    padding: spacing.md,
    borderRadius: 8,
    marginBottom: spacing.sm,
    borderWidth: 1,
    borderColor: colors.border,
  },
  settingInfo: {
    flex: 1,
    marginRight: spacing.md,
  },
  settingLabel: {
    ...typography.body,
    fontWeight: '600',
    marginBottom: spacing.xs,
  },
  settingValue: {
    ...typography.caption,
    fontFamily: 'monospace',
  },
  settingDescription: {
    ...typography.caption,
  },
  settingHint: {
    ...typography.caption,
    marginTop: spacing.xs,
    marginLeft: spacing.md,
    fontFamily: 'monospace',
    fontSize: 11,
  },
  button: {
    backgroundColor: colors.backgroundTertiary,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: 6,
    borderWidth: 1,
    borderColor: colors.border,
  },
  buttonText: {
    ...typography.body,
    fontSize: 14,
  },
  card: {
    backgroundColor: colors.backgroundSecondary,
    padding: spacing.md,
    borderRadius: 8,
    marginBottom: spacing.sm,
    borderWidth: 1,
    borderColor: colors.border,
  },
  cardLabel: {
    ...typography.caption,
    marginBottom: spacing.xs,
  },
  cardValue: {
    ...typography.body,
    fontWeight: '600',
    marginBottom: spacing.xs,
  },
  cardDescription: {
    ...typography.caption,
  },
  dangerButton: {
    backgroundColor: '#2a2a2a',
    borderColor: '#ff3b30',
  },
  dangerButtonText: {
    color: '#ff3b30',
  },
  dangerDescription: {
    ...typography.caption,
    marginTop: spacing.xs,
    color: colors.textSecondary,
    textAlign: 'center',
  },
});
