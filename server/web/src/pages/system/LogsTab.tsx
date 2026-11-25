import { useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { useLog } from '../../api/queries';
import type { LogLevel } from '../../api/types';
import { Button } from '../../components/Button';
import { EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import { Checkbox, Select } from '../../components/Form';
import { useStoredChoice } from '../../hooks/useLocalStorage';
import styles from './LogsTab.module.css';

const LEVELS: LogLevel[] = ['trace', 'debug', 'info', 'warn', 'error'];
const LINE_COUNTS = ['200', '500', '1000', '2000'] as const;

const timeFmt = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hour12: false,
});

function formatLogTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : timeFmt.format(d);
}

export function LogsTab() {
  const [level, setLevel] = useStoredChoice<LogLevel>('librisync.logs.level', LEVELS, 'info');
  const [lines, setLines] = useStoredChoice('librisync.logs.lines', LINE_COUNTS, '500');
  const [follow, setFollow] = useState(true);
  const log = useLog(Number(lines), level, follow);
  const viewport = useRef<HTMLDivElement>(null);

  // Newest is last; keep the view pinned to the bottom while following.
  useEffect(() => {
    if (follow && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  }, [log.data, follow]);

  return (
    <div className={styles.wrap}>
      <div className={styles.toolbar}>
        <Select aria-label="Minimum level" value={level} onChange={(e) => setLevel(e.target.value as LogLevel)} className={styles.select}>
          {LEVELS.map((l) => (
            <option key={l} value={l}>
              {l === 'warn' ? 'Warnings and up' : l === 'error' ? 'Errors only' : `${l[0]?.toUpperCase()}${l.slice(1)} and up`}
            </option>
          ))}
        </Select>
        <Select aria-label="Lines" value={lines} onChange={(e) => setLines(e.target.value as (typeof LINE_COUNTS)[number])} className={styles.select}>
          {LINE_COUNTS.map((n) => (
            <option key={n} value={n}>
              Last {n} lines
            </option>
          ))}
        </Select>
        <Checkbox label="Follow (refresh every 5 s)" checked={follow} onChange={(e) => setFollow(e.target.checked)} />
        <Button size="sm" icon={<RefreshCw size={14} />} onClick={() => void log.refetch()} loading={log.isFetching && !follow}>
          Refresh
        </Button>
      </div>

      {log.isPending ? (
        <SkeletonRows rows={6} label="Loading log" />
      ) : log.isError ? (
        <ErrorState error={log.error} onRetry={() => void log.refetch()} />
      ) : log.data.lines.length === 0 ? (
        <EmptyState title="No log lines at this level" />
      ) : (
        <div ref={viewport} className={styles.viewport} tabIndex={0} role="log" aria-label="Server log">
          <table className={styles.table}>
            <tbody>
              {log.data.lines.map((l, i) => (
                <tr key={i} className={styles[l.level]}>
                  <td className={styles.time}>{formatLogTime(l.time)}</td>
                  <td className={styles.level}>{l.level.toUpperCase()}</td>
                  <td className={styles.target}>{l.target}</td>
                  <td className={styles.message}>{l.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
