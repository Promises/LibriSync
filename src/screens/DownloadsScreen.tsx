import React, { useCallback, useState } from 'react';
import { View, Text, TouchableOpacity, ScrollView, Alert } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { useFocusEffect } from '@react-navigation/native';
import { useStyles } from '../hooks/useStyles';
import type { Theme } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import {
  cancelAllDownloads,
  cancelDownload,
  getStageProgress,
  listDownloadTasks,
  pauseDownload,
  resumeDownload,
  retryConversion,
  stopDownloadMonitoring,
} from '../../modules/expo-rust-bridge';
import type { DownloadTask, StageProgress, TaskStatus } from '../../modules/expo-rust-bridge';
import { getDatabasePath } from '../utils/appPaths';
import {
  formatEtaShort,
  isActiveTask,
  isBetweenParts,
  isSavedTask,
  stageLabel,
  taskPerBook,
} from '../services/downloadTasks';

const POLL_MS = 2000;
/** How far back finished and failed downloads are listed. */
const RECENT_DAYS = 7;
const RECENT_LIMIT = 30;

/** A book the service is working on (or holding in its queue). */
interface ActiveItem {
  asin: string;
  title: string;
  stage: TaskStatus;
  percentage: number;
  etaSeconds: number;
  speed?: string;
  task?: DownloadTask;
}

/** A download that ended: saved, failed or cancelled. */
interface RecentItem {
  asin: string;
  task: DownloadTask;
  endedAt: number | null;
}

/** Downloads first, then work already paused, then books still waiting. */
const STAGE_ORDER: Partial<Record<TaskStatus, number>> = { paused: 1, queued: 2 };

function toActiveItem(asin: string, task: DownloadTask | undefined, stage: StageProgress | undefined): ActiveItem {
  // The task row is the truth for pause and queue state; the service's stage store has the
  // live stage, the whole book's percentage and the speed for everything else. A finished
  // part between two parts reads as downloading.
  const status: TaskStatus =
    task?.status === 'paused' || task?.status === 'queued'
      ? task.status
      : stage?.stage ?? (task?.status === 'completed' ? 'downloading' : task?.status ?? 'queued');
  const byteShare = task && task.total_bytes > 0 ? (task.bytes_downloaded / task.total_bytes) * 100 : 0;
  return {
    asin,
    title: stage?.title ?? task?.title ?? asin,
    stage: status,
    percentage: stage?.percentage ?? byteShare,
    etaSeconds: stage?.eta_seconds ?? 0,
    speed: stage?.speed,
    task,
  };
}

function buildLists(tasks: DownloadTask[], stages: Record<string, StageProgress>) {
  const byBook = taskPerBook(tasks);
  const active: ActiveItem[] = [];
  const activeIds = new Set<string>();

  byBook.forEach((task, asin) => {
    if (isActiveTask(task) || isBetweenParts(task, stages[asin])) {
      active.push(toActiveItem(asin, task, stages[asin]));
      activeIds.add(asin);
    }
  });
  // Books the service holds without a task yet: queued behind another in sequential mode,
  // or a re-download whose newest task is still the previous, finished one.
  for (const [asin, stage] of Object.entries(stages)) {
    if (!activeIds.has(asin)) {
      active.push(toActiveItem(asin, undefined, stage));
      activeIds.add(asin);
    }
  }
  active.sort((a, b) => (STAGE_ORDER[a.stage] ?? 0) - (STAGE_ORDER[b.stage] ?? 0));

  const cutoff = Date.now() - RECENT_DAYS * 24 * 60 * 60 * 1000;
  const recent: RecentItem[] = [];
  byBook.forEach((task, asin) => {
    if (activeIds.has(asin)) return;
    if (task.status !== 'failed' && task.status !== 'cancelled' && !isSavedTask(task)) return;
    const endedAt = parseTime(task.completed_at ?? task.created_at);
    if (endedAt !== null && endedAt < cutoff) return;
    recent.push({ asin, task, endedAt });
  });
  recent.sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0));

  return { active, recent: recent.slice(0, RECENT_LIMIT) };
}

function parseTime(value: string | undefined): number | null {
  if (!value) return null;
  const t = Date.parse(value);
  return Number.isNaN(t) ? null : t;
}

function formatAgo(time: number | null): string {
  if (time === null) return '';
  const minutes = Math.round((Date.now() - time) / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

/**
 * Everything the download service is doing, in one place: books in progress with their
 * stage, progress, speed and controls, then what finished or failed in the last week.
 * The Library shows the same state per row, which is hard to follow in a big library.
 */
export default function DownloadsScreen({ navigation }: any) {
  const styles = useStyles(createStyles);
  const { colors } = useTheme();
  const [active, setActive] = useState<ActiveItem[]>([]);
  const [recent, setRecent] = useState<RecentItem[]>([]);

  const refresh = useCallback(() => {
    try {
      const lists = buildLists(listDownloadTasks(getDatabasePath()), getStageProgress());
      setActive(lists.active);
      setRecent(lists.recent);
    } catch (error) {
      console.error('[DownloadsScreen] Failed to read downloads:', error);
    }
  }, []);

  // Poll only while the tab is on screen.
  useFocusEffect(
    useCallback(() => {
      refresh();
      const timer = setInterval(refresh, POLL_MS);
      return () => clearInterval(timer);
    }, [refresh]),
  );

  const run = (label: string, action: () => void | Promise<void>) => async () => {
    try {
      await action();
    } catch (error: any) {
      Alert.alert(`${label} failed`, error?.message || 'Unknown error');
    } finally {
      refresh();
    }
  };

  const confirmCancel = (item: ActiveItem) => {
    const queued = item.stage === 'queued';
    Alert.alert(
      queued ? 'Remove from Queue' : 'Cancel Download',
      queued ? `Remove "${item.title}" from the download queue?` : `Cancel downloading "${item.title}"?`,
      [
        { text: 'No', style: 'cancel' },
        {
          text: 'Yes',
          style: 'destructive',
          onPress: run('Cancel', () => {
            // Cancel the native task if one is running, then the service-side cleanup
            // (notification, queue), which also removes a book that is only queued.
            if (item.task) cancelDownload(getDatabasePath(), item.task.task_id);
            stopDownloadMonitoring(item.asin);
          }),
        },
      ],
    );
  };

  const confirmStopAll = () => {
    Alert.alert('Stop All Downloads', 'Cancel every download in progress or waiting in the queue?', [
      { text: 'No', style: 'cancel' },
      {
        text: 'Stop All',
        style: 'destructive',
        onPress: run('Stop all', () => {
          cancelAllDownloads();
        }),
      },
    ]);
  };

  const renderActive = (item: ActiveItem) => {
    const percentage = Math.max(0, Math.min(100, item.percentage));
    const details = [
      stageLabel(item.stage),
      item.stage === 'queued' ? null : `${Math.round(percentage)}%`,
      item.speed,
      formatEtaShort(item.etaSeconds) || null,
    ].filter(Boolean).join(' · ');
    const canPause = item.task?.status === 'downloading';
    const canResume = item.task?.status === 'paused';

    return (
      <View key={item.asin} style={styles.card}>
        <View style={styles.row}>
          <View style={styles.rowText}>
            <Text style={styles.title} numberOfLines={2}>{item.title}</Text>
            <Text style={styles.caption}>{details}</Text>
          </View>
          {canPause && (
            <TouchableOpacity
              style={styles.iconButton}
              onPress={run('Pause', () => pauseDownload(getDatabasePath(), item.task!.task_id))}
              accessibilityLabel={`Pause ${item.title}`}
            >
              <Ionicons name="pause" size={20} color={colors.warning} />
            </TouchableOpacity>
          )}
          {canResume && (
            <TouchableOpacity
              style={styles.iconButton}
              onPress={run('Resume', () => resumeDownload(getDatabasePath(), item.task!.task_id))}
              accessibilityLabel={`Resume ${item.title}`}
            >
              <Ionicons name="play" size={20} color={colors.success} />
            </TouchableOpacity>
          )}
          <TouchableOpacity
            style={styles.iconButton}
            onPress={() => confirmCancel(item)}
            accessibilityLabel={`Cancel ${item.title}`}
          >
            <Ionicons name="close" size={20} color={colors.error} />
          </TouchableOpacity>
        </View>
        {item.stage !== 'queued' && (
          <View style={styles.progressTrack}>
            <View style={[styles.progressFill, { width: `${percentage}%` }]} />
          </View>
        )}
      </View>
    );
  };

  const renderRecent = ({ asin, task, endedAt }: RecentItem) => {
    const failed = task.status === 'failed';
    const saved = isSavedTask(task);
    const icon = saved ? 'checkmark-circle' : failed ? 'alert-circle' : 'close-circle';
    const iconColor = saved ? colors.success : failed ? colors.error : colors.textSecondary;
    // A conversion that failed after an Audible download can be redone from the cached
    // file; anything else is downloaded again from the Library.
    const canRetry = failed && !!task.aaxc_key;

    return (
      <View key={asin} style={styles.card}>
        <View style={styles.row}>
          <Ionicons name={icon} size={20} color={iconColor} style={styles.statusIcon} />
          <View style={styles.rowText}>
            <Text style={styles.title} numberOfLines={2}>{task.title}</Text>
            <Text style={styles.caption}>
              {[saved ? 'Downloaded' : stageLabel(task.status), formatAgo(endedAt)].filter(Boolean).join(' · ')}
            </Text>
          </View>
          {canRetry && (
            <TouchableOpacity
              style={styles.textButton}
              onPress={run('Retry', () => retryConversion(getDatabasePath(), asin))}
            >
              <Text style={styles.textButtonLabel}>Retry</Text>
            </TouchableOpacity>
          )}
        </View>
        {failed && !!task.error && (
          <Text style={styles.errorText} selectable>{task.error}</Text>
        )}
      </View>
    );
  };

  const isEmpty = active.length === 0 && recent.length === 0;

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <View style={styles.header}>
        <View style={styles.headerTitleRow}>
          {/* Pushed from Settings, so it needs its own way back. */}
          {navigation.canGoBack() && (
            <TouchableOpacity
              onPress={() => navigation.goBack()}
              style={styles.backButton}
              accessibilityLabel="Back"
            >
              <Ionicons name="chevron-back" size={24} color={colors.textPrimary} />
            </TouchableOpacity>
          )}
          <Text style={styles.headerTitle}>Downloads</Text>
        </View>
        {active.length > 0 && (
          <TouchableOpacity style={styles.headerButton} onPress={confirmStopAll}>
            <Ionicons name="stop-circle-outline" size={18} color={colors.error} />
            <Text style={styles.headerButtonText}>Stop all</Text>
          </TouchableOpacity>
        )}
      </View>

      {isEmpty ? (
        <View style={styles.empty}>
          <Ionicons name="cloud-download-outline" size={48} color={colors.textSecondary} />
          <Text style={styles.emptyTitle}>No downloads</Text>
          <Text style={styles.emptyText}>
            Books you download show up here while they download, and for a week after.
          </Text>
        </View>
      ) : (
        <ScrollView contentContainerStyle={styles.content}>
          {active.length > 0 && (
            <>
              <Text style={styles.sectionTitle}>In progress ({active.length})</Text>
              {active.map(renderActive)}
            </>
          )}
          {recent.length > 0 && (
            <>
              <Text style={styles.sectionTitle}>Recent</Text>
              {recent.map(renderRecent)}
            </>
          )}
        </ScrollView>
      )}
    </SafeAreaView>
  );
}

const createStyles = (theme: Theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  header: {
    flexDirection: 'row' as const,
    justifyContent: 'space-between' as const,
    alignItems: 'center' as const,
    padding: theme.spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  headerTitleRow: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
  },
  backButton: {
    marginRight: theme.spacing.sm,
    marginLeft: -theme.spacing.sm,
    padding: theme.spacing.xs,
  },
  headerTitle: {
    ...theme.typography.title,
  },
  headerButton: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    gap: theme.spacing.xs,
    paddingVertical: theme.spacing.xs,
    paddingHorizontal: theme.spacing.sm,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  headerButtonText: {
    ...theme.typography.caption,
    color: theme.colors.error,
  },
  content: {
    padding: theme.spacing.lg,
  },
  sectionTitle: {
    ...theme.typography.caption,
    textTransform: 'uppercase' as const,
    marginBottom: theme.spacing.sm,
    marginTop: theme.spacing.sm,
  },
  card: {
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.md,
    borderRadius: 8,
    marginBottom: theme.spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  row: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
  },
  rowText: {
    flex: 1,
    paddingRight: theme.spacing.sm,
  },
  title: {
    ...theme.typography.body,
    fontWeight: '600' as const,
  },
  caption: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
  },
  statusIcon: {
    marginRight: theme.spacing.sm,
  },
  iconButton: {
    padding: theme.spacing.sm,
    marginLeft: theme.spacing.xs,
    borderRadius: 8,
    backgroundColor: theme.colors.backgroundTertiary,
  },
  textButton: {
    paddingVertical: theme.spacing.xs,
    paddingHorizontal: theme.spacing.md,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: theme.colors.accent,
  },
  textButtonLabel: {
    ...theme.typography.caption,
    color: theme.colors.accent,
    fontWeight: '600' as const,
  },
  progressTrack: {
    height: theme.spacing.xs,
    borderRadius: theme.spacing.xs,
    backgroundColor: theme.colors.backgroundTertiary,
    marginTop: theme.spacing.sm,
    overflow: 'hidden' as const,
  },
  progressFill: {
    height: '100%' as const,
    backgroundColor: theme.colors.accent,
  },
  errorText: {
    ...theme.typography.caption,
    color: theme.colors.error,
    marginTop: theme.spacing.sm,
  },
  empty: {
    flex: 1,
    justifyContent: 'center' as const,
    alignItems: 'center' as const,
    paddingHorizontal: theme.spacing.lg,
    gap: theme.spacing.sm,
  },
  emptyTitle: {
    ...theme.typography.subtitle,
  },
  emptyText: {
    ...theme.typography.caption,
    textAlign: 'center' as const,
  },
});
