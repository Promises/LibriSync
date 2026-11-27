import { Sparkles } from 'lucide-react';
import type { BookStatus, QueueItem } from '../api/types';
import { formatPercent } from '../lib/format';
import { BOOK_STATUS } from '../lib/meta';
import { Badge } from './Badge';

/** Marks an Audible Plus catalogue title. */
export function PlusBadge() {
  return (
    <Badge
      tone="special"
      icon={<Sparkles />}
      title="Audible Plus catalogue title: downloadable only while you have an active Audible Plus membership"
    >
      Plus
    </Badge>
  );
}

/**
 * A book's status. While it is downloading, the live queue item (kept fresh by SSE)
 * supplies the percentage; a waiting item explains why in the tooltip.
 */
export function BookStatusBadge({ status, queueItem }: { status: BookStatus; queueItem?: QueueItem }) {
  const active = queueItem && (queueItem.state === 'running' || queueItem.state === 'planning');
  const meta = BOOK_STATUS[active ? 'downloading' : status];
  const label =
    active && queueItem.progress !== null ? `${meta.label} ${formatPercent(queueItem.progress)}` : meta.label;
  const title = queueItem?.state === 'waiting' && queueItem.waitingReason ? queueItem.waitingReason : meta.description;
  return (
    <Badge tone={meta.tone} icon={<meta.Icon />} title={title}>
      {label}
    </Badge>
  );
}
