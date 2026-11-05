/**
 * How the UI reads download state. Shared by the Library rows and the Downloads screen
 * so both agree on which task represents a book and whether it is still in progress.
 *
 * Two sources feed it:
 *  - the DB download tasks (`listDownloadTasks`), newest first, one per download part;
 *  - the service's in-memory stage store (`getStageProgress`), which holds the books it
 *    is working on right now (and is empty after the app process dies).
 */
import type { DownloadTask, StageProgress, TaskStatus } from '../../modules/expo-rust-bridge';

/** Statuses of a download that hasn't reached a terminal state yet. */
const ACTIVE_TASK_STATUSES: ReadonlySet<TaskStatus> = new Set<TaskStatus>([
  'queued', 'downloading', 'paused', 'decrypting', 'joining', 'validating', 'copying', 'encoding',
]);

export const isActiveTask = (task: DownloadTask): boolean => ACTIVE_TASK_STATUSES.has(task.status);

/**
 * The task that represents each book. `tasks` is newest-first, so the first task seen
 * per book wins — except that an in-flight task beats a finished one: a book in parts
 * (Chirp's tracks) leaves a "completed" row per finished part while the next part runs.
 */
export function taskPerBook(tasks: DownloadTask[]): Map<string, DownloadTask> {
  const byBook = new Map<string, DownloadTask>();
  for (const task of tasks) {
    const current = byBook.get(task.asin);
    if (!current || (isActiveTask(task) && !isActiveTask(current))) {
      byBook.set(task.asin, task);
    }
  }
  return byBook;
}

/** The book's file is saved: only the finalize step gives a task an output path. */
export const isSavedTask = (task: DownloadTask | undefined): boolean =>
  task?.status === 'completed' && !!task.output_path;

/**
 * A "completed" task without a saved file is one finished part of a book that is still in
 * progress — as long as the service is working on it. Without a live stage entry the book
 * was interrupted (e.g. the app was killed) and should be offered for download again.
 */
export const isBetweenParts = (
  task: DownloadTask | undefined,
  stage: StageProgress | undefined,
): boolean => task?.status === 'completed' && !task.output_path && !!stage;

/** Human-readable name of a stage, for lists and headings. */
export function stageLabel(status: TaskStatus | 'between_parts'): string {
  switch (status) {
    case 'queued': return 'Queued';
    case 'downloading':
    case 'between_parts': return 'Downloading';
    case 'paused': return 'Paused';
    case 'decrypting': return 'Decrypting';
    case 'joining': return 'Joining tracks';
    case 'validating': return 'Validating';
    case 'copying': return 'Saving';
    case 'encoding': return 'Converting to MP3';
    case 'completed': return 'Downloaded';
    case 'failed': return 'Failed';
    case 'cancelled': return 'Cancelled';
    default: return status;
  }
}

/** Remaining time as "~1h 5m" / "~3m 20s" / "~12s"; empty when unknown. */
export function formatEtaShort(sec: number): string {
  if (!sec || sec <= 0) return '';
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  if (h > 0) return `~${h}h ${m}m`;
  if (m > 0) return `~${m}m ${s}s`;
  return `~${s}s`;
}
