import React, { useState, useEffect } from 'react';
import { View, Text, ScrollView, RefreshControl, TouchableOpacity, Alert } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import {
  getActiveTasks,
  startBackgroundService,
  stopBackgroundService,
  pauseTask,
  resumeTask,
  cancelTask,
  enableAutoDownload,
  disableAutoDownload,
  enableAutoSync,
  disableAutoSync,
  startLibrarySyncNew,
  getPrimaryAccount,
  initializeDatabase,
  clearLibrary,
  clearDownloadState,
  clearAllTasks,
  isBackgroundServiceRunning,
  type BackgroundTask,
} from '../../modules/expo-rust-bridge';
import { Directory, File, Paths } from 'expo-file-system';
import Button from '../components/Button';

/**
 * Task Debug Screen
 *
 * Comprehensive debugging interface for the BackgroundTaskManager system.
 *
 * Features:
 * - Live task list with auto-refresh
 * - Task details (type, priority, status, metadata)
 * - Task control buttons (pause/resume/cancel)
 * - Service controls (start service, enable/disable auto-download)
 * - Test actions (trigger sync, test downloads)
 */
export default function TaskDebugScreen() {
  const styles = useStyles(createStyles);
  const { colors, spacing } = useTheme();

  const [tasks, setTasks] = useState<BackgroundTask[]>([]);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [lastRefresh, setLastRefresh] = useState<Date>(new Date());
  const [serviceStarted, setServiceStarted] = useState(false);
  const [autoDownloadEnabled, setAutoDownloadEnabled] = useState(false);
  const [autoSyncEnabled, setAutoSyncEnabled] = useState(true); // Default: enabled
  const [hasAccount, setHasAccount] = useState<boolean | null>(null); // null = checking

  // Auto-refresh every 2 seconds
  useEffect(() => {
    const interval = setInterval(() => {
      refreshTasks();
      checkServiceStatus();
    }, 2000);

    // Initial refresh
    refreshTasks();
    checkAccount();
    checkServiceStatus();

    return () => clearInterval(interval);
  }, []);

  const checkAccount = async () => {
    try {
      const cacheUri = Paths.cache.uri;
      const cachePath = cacheUri.replace('file://', '');
      const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

      initializeDatabase(dbPath);
      const account = await getPrimaryAccount(dbPath);
      setHasAccount(account !== null);
    } catch (error) {
      console.error('[TaskDebug] Error checking account:', error);
      setHasAccount(false);
    }
  };

  const checkServiceStatus = () => {
    try {
      const isRunning = isBackgroundServiceRunning();
      setServiceStarted(isRunning);
    } catch (error) {
      console.error('[TaskDebug] Error checking service status:', error);
      setServiceStarted(false);
    }
  };

  const refreshTasks = () => {
    try {
      const activeTasks = getActiveTasks();
      setTasks(activeTasks);
      setLastRefresh(new Date());
    } catch (error) {
      console.error('[TaskDebug] Error getting active tasks:', error);
    }
  };

  const handleRefresh = async () => {
    setIsRefreshing(true);
    refreshTasks();
    setIsRefreshing(false);
  };

  const handleStartService = () => {
    try {
      startBackgroundService();
      setServiceStarted(true);
      Alert.alert('Success', 'Background service started');
    } catch (error: any) {
      Alert.alert('Error', error.message || 'Failed to start service');
    }
  };

  const handleStopService = () => {
    Alert.alert(
      'Stop Background Service?',
      'This will stop all automatic features:\n\n• Automatic token refresh\n• Automatic library sync\n• Auto-downloads\n\nYou can restart it anytime by reopening the app.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Stop Service',
          style: 'destructive',
          onPress: () => {
            try {
              stopBackgroundService();
              setServiceStarted(false);
              Alert.alert('Stopped', 'Background service stopped. Notification removed.');
            } catch (error: any) {
              Alert.alert('Error', error.message || 'Failed to stop service');
            }
          },
        },
      ]
    );
  };

  const handleStartSync = async () => {
    if (!hasAccount) {
      Alert.alert(
        'No Account',
        'Please log in via the Account tab before syncing your library.',
        [{ text: 'OK' }]
      );
      return;
    }

    try {
      await startLibrarySyncNew(false);
      Alert.alert('Success', 'Library sync started');
      refreshTasks();
    } catch (error: any) {
      Alert.alert('Error', error.message || 'Failed to start sync');
    }
  };

  const handleClearDownloadState = async () => {
    Alert.alert(
      'Clear Download State?',
      'This will reset download status for all books (mark as not downloaded). Book metadata will be kept. Continue?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Clear Download State',
          style: 'destructive',
          onPress: async () => {
            try {
              const cacheUri = Paths.cache.uri;
              const cachePath = cacheUri.replace('file://', '');
              const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

              const booksUpdated = await clearDownloadState(dbPath);
              Alert.alert('Success', `Download state cleared for ${booksUpdated} books.`);
            } catch (error: any) {
              Alert.alert('Error', error.message || 'Failed to clear download state');
            }
          },
        },
      ]
    );
  };

  const handleClearLibrary = async () => {
    Alert.alert(
      'Clear Library?',
      'This will:\n• Delete ALL books from the database\n• Reset download state for all books\n• Cancel all active download tasks\n\nThis is irreversible and meant for testing only. Continue?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Clear Library',
          style: 'destructive',
          onPress: async () => {
            try {
              const cacheUri = Paths.cache.uri;
              const cachePath = cacheUri.replace('file://', '');
              const dbPath = `${cachePath.replace(/\/$/, '')}/audible.db`;

              // Clear download state first (before deleting books)
              const booksUpdated = await clearDownloadState(dbPath);

              // Clear all active download tasks
              clearAllTasks();

              // Clear library from database
              await clearLibrary(dbPath);

              Alert.alert(
                'Success',
                `Library cleared:\n• ${booksUpdated} books had download state reset\n• All books deleted from database\n• All download tasks cancelled`
              );
              checkAccount(); // Re-check account status
              refreshTasks(); // Refresh task list
            } catch (error: any) {
              Alert.alert('Error', error.message || 'Failed to clear library');
            }
          },
        },
      ]
    );
  };

  const handleClearAllTasks = () => {
    Alert.alert(
      'Clear All Tasks?',
      'This will remove all active, pending, and stuck tasks. This is useful for recovering from stuck states. Continue?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Clear Tasks',
          style: 'destructive',
          onPress: () => {
            try {
              clearAllTasks();
              Alert.alert('Success', 'All tasks cleared');
              refreshTasks();
            } catch (error: any) {
              Alert.alert('Error', error.message || 'Failed to clear tasks');
            }
          },
        },
      ]
    );
  };

  const handleToggleAutoDownload = () => {
    try {
      if (autoDownloadEnabled) {
        disableAutoDownload();
        setAutoDownloadEnabled(false);
        Alert.alert('Success', 'Auto-download disabled');
      } else {
        enableAutoDownload();
        setAutoDownloadEnabled(true);
        Alert.alert('Success', 'Auto-download enabled');
      }
    } catch (error: any) {
      Alert.alert('Error', error.message || 'Failed to toggle auto-download');
    }
  };

  const handleToggleAutoSync = () => {
    try {
      if (autoSyncEnabled) {
        disableAutoSync();
        setAutoSyncEnabled(false);
        Alert.alert('Success', 'Auto-sync disabled');
      } else {
        enableAutoSync(24); // Default: daily
        setAutoSyncEnabled(true);
        Alert.alert('Success', 'Auto-sync enabled (checks daily)');
      }
    } catch (error: any) {
      Alert.alert('Error', error.message || 'Failed to toggle auto-sync');
    }
  };

  const handlePauseTask = async (taskId: string) => {
    try {
      const success = await pauseTask(taskId);
      if (success) {
        Alert.alert('Success', 'Task paused');
        refreshTasks();
      } else {
        Alert.alert('Error', 'Failed to pause task');
      }
    } catch (error: any) {
      Alert.alert('Error', error.message || 'Failed to pause task');
    }
  };

  const handleResumeTask = async (taskId: string) => {
    try {
      const success = await resumeTask(taskId);
      if (success) {
        Alert.alert('Success', 'Task resumed');
        refreshTasks();
      } else {
        Alert.alert('Error', 'Failed to resume task');
      }
    } catch (error: any) {
      Alert.alert('Error', error.message || 'Failed to resume task');
    }
  };

  const handleCancelTask = async (taskId: string) => {
    Alert.alert(
      'Cancel Task',
      'Are you sure you want to cancel this task?',
      [
        { text: 'No', style: 'cancel' },
        {
          text: 'Yes',
          style: 'destructive',
          onPress: async () => {
            try {
              const success = await cancelTask(taskId);
              if (success) {
                Alert.alert('Success', 'Task cancelled');
                refreshTasks();
              } else {
                Alert.alert('Error', 'Failed to cancel task');
              }
            } catch (error: any) {
              Alert.alert('Error', error.message || 'Failed to cancel task');
            }
          },
        },
      ]
    );
  };

  const handleListCacheFiles = async () => {
    try {
      console.log('[TaskDebug] Listing cache directory recursively...');
      console.log('[TaskDebug] Cache path:', Paths.cache.uri);

      const cacheDir = new Directory(Paths.cache);

      if (!cacheDir.exists) {
        console.log('[TaskDebug] Cache directory does not exist');
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
              console.log('[TaskDebug]   ', logEntry);

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
            console.log('[TaskDebug]   ', logEntry);
          }
        }
      };

      await listRecursive(cacheDir);

      console.log('[TaskDebug] ================');
      console.log('[TaskDebug] Total:', fileCount, 'files,', dirCount, 'directories');
      console.log('[TaskDebug] Total size:', (totalSize / 1024 / 1024).toFixed(2), 'MB');

      // Show summary in alert
      const summary = fileDetails.slice(0, 15).join('\n');
      const more = fileDetails.length > 15 ? `\n... and ${fileDetails.length - 15} more` : '';
      const totalSizeMB = (totalSize / 1024 / 1024).toFixed(2);
      Alert.alert(
        'Cache Directory',
        `${fileCount} files, ${dirCount} dirs\nTotal: ${totalSizeMB} MB\n\n${summary}${more}\n\nCheck console for full listing.`
      );
    } catch (error: any) {
      console.error('[TaskDebug] Failed to list cache files:', error);
      Alert.alert('Error', error.message || 'Failed to list cache directory');
    }
  };

  const formatDate = (timestamp: number | undefined) => {
    if (!timestamp) return 'N/A';
    const date = new Date(timestamp);
    return date.toLocaleTimeString();
  };

  const formatDuration = (start: number | undefined, end: number | undefined) => {
    if (!start) return 'N/A';
    const endTime = end || Date.now();
    const duration = endTime - start;
    const seconds = Math.floor(duration / 1000);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
    const hours = Math.floor(minutes / 60);
    return `${hours}h ${minutes % 60}m`;
  };

  const getStatusColor = (status: string) => {
    switch (status) {
      case 'RUNNING': return colors.info;
      case 'COMPLETED': return colors.success;
      case 'FAILED': return colors.error;
      case 'PAUSED': return colors.warning;
      case 'CANCELLED': return colors.textSecondary;
      default: return colors.textSecondary;
    }
  };

  const renderTask = (task: BackgroundTask) => {
    const isDownload = task.type === 'DOWNLOAD';
    const isRunning = task.status === 'RUNNING';
    const isPaused = task.status === 'PAUSED';

    return (
      <View key={task.id} style={styles.taskCard}>
        {/* Task Header */}
        <View style={styles.taskHeader}>
          <View style={styles.taskInfo}>
            <Text style={styles.taskType}>{task.type}</Text>
            <View style={[styles.statusBadge, { backgroundColor: getStatusColor(task.status) }]}>
              <Text style={styles.statusText}>{task.status}</Text>
            </View>
          </View>
          <Text style={styles.taskPriority}>{task.priority}</Text>
        </View>

        {/* Task ID */}
        <Text style={styles.taskId}>ID: {task.id}</Text>

        {/* Download-specific info */}
        {isDownload && task.metadata.asin && (
          <View style={styles.metadataSection}>
            <Text style={styles.metadataLabel}>BOOK:</Text>
            <Text style={styles.metadataValue}>{task.metadata.title || task.metadata.asin}</Text>
            {task.metadata.author && (
              <Text style={styles.metadataValue}>by {task.metadata.author}</Text>
            )}
          </View>
        )}

        {/* Progress for downloads */}
        {isDownload && task.metadata.percentage !== undefined && (
          <View style={styles.progressSection}>
            <View style={styles.progressBar}>
              <View
                style={[
                  styles.progressFill,
                  {
                    width: `${task.metadata.percentage || 0}%`,
                    backgroundColor: colors.info,
                  },
                ]}
              />
            </View>
            <Text style={styles.progressText}>
              {task.metadata.percentage || 0}% - {task.metadata.stage || 'preparing'}
            </Text>
            {task.metadata.bytes_downloaded && task.metadata.total_bytes && (
              <Text style={styles.progressDetails}>
                {Math.round(task.metadata.bytes_downloaded / 1024 / 1024)} /{' '}
                {Math.round(task.metadata.total_bytes / 1024 / 1024)} MB
              </Text>
            )}
          </View>
        )}

        {/* Sync-specific info */}
        {task.type === 'LIBRARY_SYNC' && (
          <View style={styles.metadataSection}>
            {task.metadata.current_page !== undefined && (
              <Text style={styles.metadataValue}>Page: {task.metadata.current_page}</Text>
            )}
            {task.metadata.items_synced !== undefined && (
              <Text style={styles.metadataValue}>
                Synced: {task.metadata.items_synced} items ({task.metadata.items_added || 0} new)
              </Text>
            )}
          </View>
        )}

        {/* Timestamps */}
        <View style={styles.timestampSection}>
          <Text style={styles.timestamp}>Created: {formatDate(task.createdAt)}</Text>
          {task.startedAt && (
            <Text style={styles.timestamp}>Started: {formatDate(task.startedAt)}</Text>
          )}
          {task.completedAt && (
            <Text style={styles.timestamp}>Completed: {formatDate(task.completedAt)}</Text>
          )}
          {task.startedAt && !task.completedAt && (
            <Text style={styles.timestamp}>Duration: {formatDuration(task.startedAt, task.completedAt)}</Text>
          )}
        </View>

        {/* Error */}
        {task.error && (
          <View style={styles.errorSection}>
            <Text style={styles.errorLabel}>ERROR:</Text>
            <Text style={styles.errorText}>{task.error}</Text>
          </View>
        )}

        {/* Control Buttons */}
        <View style={styles.controlButtons}>
          {isRunning && (
            <TouchableOpacity
              style={[styles.controlButton, { backgroundColor: colors.warning }]}
              onPress={() => handlePauseTask(task.id)}
            >
              <Text style={styles.controlButtonText}>Pause</Text>
            </TouchableOpacity>
          )}
          {isPaused && (
            <TouchableOpacity
              style={[styles.controlButton, { backgroundColor: colors.info }]}
              onPress={() => handleResumeTask(task.id)}
            >
              <Text style={styles.controlButtonText}>Resume</Text>
            </TouchableOpacity>
          )}
          {(isRunning || isPaused) && (
            <TouchableOpacity
              style={[styles.controlButton, { backgroundColor: colors.error }]}
              onPress={() => handleCancelTask(task.id)}
            >
              <Text style={styles.controlButtonText}>Cancel</Text>
            </TouchableOpacity>
          )}
        </View>

        {/* Raw Metadata (collapsed by default) */}
        <View style={styles.rawMetadata}>
          <Text style={styles.rawMetadataLabel}>Metadata:</Text>
          <Text style={styles.rawMetadataText}>{JSON.stringify(task.metadata, null, 2)}</Text>
        </View>
      </View>
    );
  };

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <ScrollView
        style={styles.scrollView}
        refreshControl={
          <RefreshControl refreshing={isRefreshing} onRefresh={handleRefresh} />
        }
      >
        {/* Header */}
        <View style={styles.header}>
          <Text style={styles.title}>Task Debug</Text>
          <Text style={styles.subtitle}>
            {tasks.length} active tasks • Last refresh: {lastRefresh.toLocaleTimeString()}
          </Text>
        </View>

        {/* Service Controls */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Service Controls</Text>

          <View style={[styles.statusCard, { borderColor: serviceStarted ? colors.success : colors.textSecondary }]}>
            <Text style={styles.statusLabel}>Background Service:</Text>
            <Text style={[styles.statusValue, { color: serviceStarted ? colors.success : colors.textSecondary }]}>
              {serviceStarted ? '✓ RUNNING' : '○ STOPPED'}
            </Text>
            <Text style={styles.caption}>
              {serviceStarted
                ? 'Service runs when there is active work (downloads, sync, etc.)'
                : 'Service stopped - no active work'}
            </Text>
          </View>

          {serviceStarted && (
            <Button
              title="Stop Service (Remove Notification)"
              onPress={handleStopService}
              variant="outlined"
              state="error"
              style={{ marginBottom: spacing.sm }}
            />
          )}
          {!serviceStarted && (
            <Button
              title="Start Service"
              onPress={handleStartService}
              variant="outlined"
              state="success"
              style={{ marginBottom: spacing.sm }}
            />
          )}

          <Button
            title={autoDownloadEnabled ? 'Disable Auto-Download' : 'Enable Auto-Download'}
            onPress={handleToggleAutoDownload}
            variant="outlined"
            state={autoDownloadEnabled ? 'error' : 'success'}
            style={{ marginBottom: spacing.sm }}
          />

          <Button
            title={autoSyncEnabled ? 'Disable Auto-Sync (Daily)' : 'Enable Auto-Sync (Daily)'}
            onPress={handleToggleAutoSync}
            variant="outlined"
            state={autoSyncEnabled ? 'error' : 'success'}
          />
        </View>

        {/* Account Status */}
        {hasAccount === false && (
          <View style={styles.section}>
            <View style={[styles.statusCard, { borderColor: colors.warning, backgroundColor: colors.warning + '20' }]}>
              <Text style={styles.statusLabel}>⚠️ No Account Configured</Text>
              <Text style={[styles.statusValue, { color: colors.warning }]}>
                Please log in via the Account tab to enable background tasks
              </Text>
            </View>
          </View>
        )}

        {/* Test Actions */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Test Actions</Text>

          <Button
            title="Start Library Sync"
            onPress={handleStartSync}
            variant="outlined"
            state="warning"
            disabled={hasAccount === false}
            style={{ marginBottom: spacing.sm }}
          />

          <Button
            title="List Cache Files"
            onPress={handleListCacheFiles}
            variant="outlined"
            state="primary"
            style={{ marginBottom: spacing.sm }}
          />

          <Button
            title="Clear All Tasks"
            onPress={handleClearAllTasks}
            variant="outlined"
            state="error"
            style={{ marginBottom: spacing.sm }}
          />

          <Button
            title="Clear Download State (Reset Downloaded)"
            onPress={handleClearDownloadState}
            variant="outlined"
            state="error"
            style={{ marginBottom: spacing.sm }}
          />

          <Button
            title="Clear Library (Delete All Books)"
            onPress={handleClearLibrary}
            variant="outlined"
            state="error"
            style={{ marginBottom: spacing.sm }}
          />

          <Button
            title="Refresh Task List"
            onPress={handleRefresh}
            variant="outlined"
            state="primary"
          />
        </View>

        {/* Active Tasks */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Active Tasks</Text>

          {tasks.length === 0 ? (
            <View style={styles.emptyState}>
              <Text style={styles.emptyStateText}>No active tasks</Text>
              <Text style={styles.emptyStateHint}>
                Start a download or library sync to see tasks here
              </Text>
            </View>
          ) : (
            tasks.map(renderTask)
          )}
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}

const createStyles = (theme: Theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  scrollView: {
    flex: 1,
  },
  header: {
    padding: theme.spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  title: {
    ...theme.typography.title,
    marginBottom: theme.spacing.xs,
  },
  subtitle: {
    ...theme.typography.caption,
  },
  section: {
    padding: theme.spacing.lg,
  },
  sectionTitle: {
    ...theme.typography.subtitle,
    marginBottom: theme.spacing.md,
    textTransform: 'uppercase' as const,
  },
  statusCard: {
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.md,
    borderRadius: 8,
    borderWidth: 2,
    marginBottom: theme.spacing.sm,
  },
  statusLabel: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.xs,
  },
  statusValue: {
    ...theme.typography.subtitle,
    fontWeight: '700' as const,
  },
  caption: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
    marginTop: theme.spacing.xs,
  },
  taskCard: {
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.md,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: theme.colors.border,
    marginBottom: theme.spacing.md,
  },
  taskHeader: {
    flexDirection: 'row' as const,
    justifyContent: 'space-between' as const,
    alignItems: 'center' as const,
    marginBottom: theme.spacing.sm,
  },
  taskInfo: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    gap: theme.spacing.sm,
  },
  taskType: {
    ...theme.typography.subtitle,
    fontWeight: '700' as const,
  },
  statusBadge: {
    paddingHorizontal: theme.spacing.sm,
    paddingVertical: theme.spacing.xs / 2,
    borderRadius: 4,
  },
  statusText: {
    ...theme.typography.caption,
    color: theme.colors.background,
    fontWeight: '700' as const,
    fontSize: 10,
  },
  taskPriority: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
  },
  taskId: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
    marginBottom: theme.spacing.sm,
    fontFamily: theme.typography.mono.fontFamily,
  },
  metadataSection: {
    marginBottom: theme.spacing.sm,
  },
  metadataLabel: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.xs,
  },
  metadataValue: {
    ...theme.typography.body,
  },
  progressSection: {
    marginBottom: theme.spacing.sm,
  },
  progressBar: {
    height: 8,
    backgroundColor: theme.colors.border,
    borderRadius: 4,
    overflow: 'hidden' as const,
    marginBottom: theme.spacing.xs,
  },
  progressFill: {
    height: '100%',
    borderRadius: 4,
  },
  progressText: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.xs / 2,
  },
  progressDetails: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
  },
  timestampSection: {
    marginBottom: theme.spacing.sm,
  },
  timestamp: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
  },
  errorSection: {
    backgroundColor: theme.colors.error + '20',
    padding: theme.spacing.sm,
    borderRadius: 4,
    marginBottom: theme.spacing.sm,
  },
  errorLabel: {
    ...theme.typography.caption,
    color: theme.colors.error,
    fontWeight: '700' as const,
    marginBottom: theme.spacing.xs,
  },
  errorText: {
    ...theme.typography.caption,
    color: theme.colors.error,
  },
  controlButtons: {
    flexDirection: 'row' as const,
    gap: theme.spacing.sm,
    marginBottom: theme.spacing.sm,
  },
  controlButton: {
    flex: 1,
    paddingVertical: theme.spacing.sm,
    paddingHorizontal: theme.spacing.md,
    borderRadius: 4,
    alignItems: 'center' as const,
  },
  controlButtonText: {
    ...theme.typography.caption,
    color: theme.colors.background,
    fontWeight: '700' as const,
  },
  rawMetadata: {
    backgroundColor: theme.colors.background,
    padding: theme.spacing.sm,
    borderRadius: 4,
    marginTop: theme.spacing.sm,
  },
  rawMetadataLabel: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.xs,
  },
  rawMetadataText: {
    ...theme.typography.mono,
    fontSize: 10,
    color: theme.colors.textSecondary,
  },
  emptyState: {
    padding: theme.spacing.xl,
    alignItems: 'center' as const,
  },
  emptyStateText: {
    ...theme.typography.body,
    marginBottom: theme.spacing.sm,
  },
  emptyStateHint: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
    textAlign: 'center' as const,
  },
});
