import { AlertCircle, AlertTriangle, CheckCircle2, Info } from 'lucide-react';
import { useAccounts, useHealth, useSystemStatus } from '../../api/queries';
import type { AccountView, HealthItem } from '../../api/types';
import { Card } from '../../components/Card';
import { ErrorState, SkeletonRows } from '../../components/Feedback';
import { formatDateTime, formatNumber, formatRelative } from '../../lib/format';
import styles from './StatusTab.module.css';

const HEALTH_ICON = { ok: CheckCircle2, notice: Info, warning: AlertTriangle, error: AlertCircle } as const;
const HEALTH_RANK = { error: 0, warning: 1, notice: 2, ok: 3 } as const;

/** "account:<id>" → the account's name; other sources get a readable name. */
function sourceLabel(source: string, accounts: AccountView[] | undefined): string {
  if (source.startsWith('account:')) {
    const id = source.slice('account:'.length);
    return `Account: ${accounts?.find((a) => a.id === id)?.name ?? id}`;
  }
  const names: Record<string, string> = {
    ffmpeg: 'FFmpeg',
    libraryDir: 'Library folder',
    disk: 'Disk space',
    notifications: 'Notifications',
  };
  return names[source] ?? source;
}

function HealthList({ items }: { items: HealthItem[] }) {
  const accounts = useAccounts();
  const problems = items.filter((h) => h.type !== 'ok').sort((a, b) => HEALTH_RANK[a.type] - HEALTH_RANK[b.type]);
  if (problems.length === 0) {
    return (
      <p className={styles.allGood}>
        <CheckCircle2 size={18} aria-hidden /> No problems found.
      </p>
    );
  }
  return (
    <ul className={styles.health}>
      {problems.map((h, i) => {
        const Icon = HEALTH_ICON[h.type];
        return (
          <li key={`${h.source}-${i}`} className={`${styles.healthItem} ${styles[h.type]}`}>
            <Icon size={18} aria-hidden className={styles.healthIcon} />
            <div>
              <p className={styles.healthSource}>{sourceLabel(h.source, accounts.data)}</p>
              <p>{h.message}</p>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

export function StatusTab() {
  const status = useSystemStatus();
  const health = useHealth();

  return (
    <div className={styles.grid}>
      <Card title="Health">
        {health.isPending ? (
          <SkeletonRows rows={2} label="Checking health" />
        ) : health.isError ? (
          <ErrorState error={health.error} onRetry={() => void health.refetch()} />
        ) : (
          <HealthList items={health.data} />
        )}
      </Card>

      <Card title="About">
        {status.isPending ? (
          <SkeletonRows rows={4} label="Loading status" />
        ) : status.isError ? (
          <ErrorState error={status.error} onRetry={() => void status.refetch()} />
        ) : (
          <dl className={styles.facts}>
            <div>
              <dt>Version</dt>
              <dd>{status.data.version}</dd>
            </div>
            <div>
              <dt>Started</dt>
              <dd title={formatDateTime(status.data.startTime)}>{formatRelative(status.data.startTime)}</dd>
            </div>
            <div>
              <dt>FFmpeg</dt>
              <dd>{status.data.ffmpegVersion ?? <span className={styles.missing}>Not found</span>}</dd>
            </div>
            <div>
              <dt>Platform</dt>
              <dd>
                {status.data.os}
                {status.data.isDocker ? ' (Docker)' : ''}
              </dd>
            </div>
            <div>
              <dt>Config folder</dt>
              <dd className={styles.path}>{status.data.configDir}</dd>
            </div>
            <div>
              <dt>Library folder</dt>
              <dd className={styles.path}>{status.data.libraryDir}</dd>
            </div>
          </dl>
        )}
      </Card>

      {status.data && (
        <Card title="Library">
          <dl className={styles.counts}>
            <div>
              <dt>Accounts</dt>
              <dd>{formatNumber(status.data.accounts)}</dd>
            </div>
            <div>
              <dt>Books</dt>
              <dd>{formatNumber(status.data.books)}</dd>
            </div>
            <div>
              <dt>Downloaded</dt>
              <dd>{formatNumber(status.data.downloaded)}</dd>
            </div>
            <div>
              <dt>In queue</dt>
              <dd>{formatNumber(status.data.queued)}</dd>
            </div>
          </dl>
        </Card>
      )}
    </div>
  );
}
