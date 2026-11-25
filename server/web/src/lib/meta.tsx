import type { ComponentType } from 'react';
import {
  Ban,
  CheckCircle2,
  CircleDashed,
  CirclePause,
  CloudDownload,
  Download,
  Hourglass,
  KeyRound,
  Library,
  ListPlus,
  RefreshCw,
  ShieldAlert,
  ShieldX,
  Snowflake,
  UserMinus,
  UserPlus,
  XCircle,
  type LucideProps,
} from 'lucide-react';
import type { BookStatus, HistoryEvent, NotificationEvent, QueuePriority, QueueStage, QueueState } from '../api/types';
import type { Tone } from '../components/Badge';

type Meta = { label: string; tone: Tone; Icon: ComponentType<LucideProps> };

export const BOOK_STATUS: Record<BookStatus, Meta & { description: string }> = {
  downloaded: { label: 'Downloaded', tone: 'success', Icon: CheckCircle2, description: 'In the library folder' },
  downloading: { label: 'Downloading', tone: 'accent', Icon: CloudDownload, description: 'Being downloaded now' },
  queued: { label: 'Queued', tone: 'info', Icon: ListPlus, description: 'Waiting in the download queue' },
  wanted: { label: 'Wanted', tone: 'neutral', Icon: CircleDashed, description: 'Not downloaded yet; back-fill will get to it' },
  cooldown: { label: 'Cooldown', tone: 'warning', Icon: Snowflake, description: 'Waiting out a licence refusal before retrying' },
  failed: { label: 'Failed', tone: 'danger', Icon: XCircle, description: 'The last attempt failed' },
  skipped: { label: 'Skipped', tone: 'neutral', Icon: Ban, description: 'Marked as not wanted; back-fill ignores it' },
};

export const BOOK_STATUS_ORDER: BookStatus[] = ['downloaded', 'downloading', 'queued', 'wanted', 'cooldown', 'failed', 'skipped'];

export const QUEUE_STATE: Record<QueueState, Meta> = {
  queued: { label: 'Queued', tone: 'info', Icon: ListPlus },
  waiting: { label: 'Waiting', tone: 'warning', Icon: Hourglass },
  planning: { label: 'Requesting licence', tone: 'accent', Icon: KeyRound },
  running: { label: 'Running', tone: 'accent', Icon: CloudDownload },
  completed: { label: 'Completed', tone: 'success', Icon: CheckCircle2 },
  failed: { label: 'Failed', tone: 'danger', Icon: XCircle },
  cancelled: { label: 'Cancelled', tone: 'neutral', Icon: CirclePause },
};

export const QUEUE_STAGE: Record<QueueStage, string> = {
  downloading: 'Downloading',
  decrypting: 'Decrypting',
  extracting: 'Extracting',
  joining: 'Joining tracks',
  validating: 'Validating',
  encoding: 'Encoding MP3',
  finalizing: 'Finalizing',
};

export const PRIORITY: Record<QueuePriority, { label: string; tone: Tone; description: string }> = {
  manual: { label: 'Manual', tone: 'accent', description: 'Requested by you; ignores licence spacing' },
  new: { label: 'New', tone: 'info', description: 'A new purchase; ignores licence spacing' },
  backfill: { label: 'Back-fill', tone: 'neutral', description: 'Old library; paced by the licence budget' },
};

export const HISTORY_EVENT: Record<HistoryEvent, Meta> = {
  bookAdded: { label: 'Book added', tone: 'info', Icon: Library },
  grab: { label: 'Grabbed', tone: 'accent', Icon: Download },
  download: { label: 'Downloaded', tone: 'success', Icon: CheckCircle2 },
  downloadFailed: { label: 'Download failed', tone: 'danger', Icon: XCircle },
  licenceDenied: { label: 'Licence denied', tone: 'danger', Icon: ShieldX },
  licenceThrottled: { label: 'Licence throttled', tone: 'warning', Icon: ShieldAlert },
  syncCompleted: { label: 'Sync completed', tone: 'success', Icon: RefreshCw },
  syncFailed: { label: 'Sync failed', tone: 'danger', Icon: RefreshCw },
  accountAdded: { label: 'Account added', tone: 'info', Icon: UserPlus },
  accountRemoved: { label: 'Account removed', tone: 'neutral', Icon: UserMinus },
  authExpired: { label: 'Sign-in expired', tone: 'warning', Icon: KeyRound },
};

/** Labels for the notification event checkboxes: every history event plus health changes. */
export const NOTIFICATION_EVENT_LABEL: Record<NotificationEvent, string> = {
  ...(Object.fromEntries(Object.entries(HISTORY_EVENT).map(([k, v]) => [k, v.label])) as Record<HistoryEvent, string>),
  health: 'Health issue',
  healthRestored: 'Health restored',
};
