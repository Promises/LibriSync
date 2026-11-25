import type { HistoryEvent } from '../api/types';
import { HISTORY_EVENT } from '../lib/meta';
import styles from './EventIcon.module.css';

/** The round, tinted icon for a history event. */
export function EventIcon({ event }: { event: HistoryEvent }) {
  const meta = HISTORY_EVENT[event];
  return (
    <span className={`${styles.icon} ${styles[meta.tone]}`} title={meta.label}>
      <meta.Icon size={14} aria-hidden />
    </span>
  );
}
