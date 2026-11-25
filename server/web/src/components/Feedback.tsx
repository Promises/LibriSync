import type { ReactNode } from 'react';
import { AlertCircle, AlertTriangle, CheckCircle2, Info, RotateCw } from 'lucide-react';
import { errorMessage } from '../api/client';
import { Button } from './Button';
import styles from './Feedback.module.css';

export function EmptyState({
  icon,
  title,
  children,
  action,
}: {
  icon?: ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className={styles.empty}>
      {icon && <div className={styles.emptyIcon}>{icon}</div>}
      <h3 className={styles.emptyTitle}>{title}</h3>
      {children && <div className={styles.emptyText}>{children}</div>}
      {action && <div className={styles.emptyAction}>{action}</div>}
    </div>
  );
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  return (
    <div className={styles.errorState} role="alert">
      <AlertCircle size={20} aria-hidden />
      <div className={styles.errorText}>
        <p className={styles.errorTitle}>Couldn't load this</p>
        <p>{errorMessage(error)}</p>
      </div>
      {onRetry && (
        <Button size="sm" icon={<RotateCw size={14} />} onClick={onRetry}>
          Retry
        </Button>
      )}
    </div>
  );
}

/** Placeholder block shown while data loads. */
export function Skeleton({ width, height = 14, round = false }: { width?: string | number; height?: number; round?: boolean }) {
  return (
    <span
      className={round ? `${styles.skeleton} ${styles.skeletonRound}` : styles.skeleton}
      style={{ width: width ?? '100%', height }}
      aria-hidden
    />
  );
}

/** A stack of skeleton rows sized like a list or table. */
export function SkeletonRows({ rows = 5, label = 'Loading' }: { rows?: number; label?: string }) {
  return (
    <div className={styles.skeletonRows} role="status" aria-label={label}>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className={styles.skeletonRow}>
          <Skeleton width={40} height={40} />
          <div className={styles.skeletonLines}>
            <Skeleton width={`${60 - (i % 3) * 12}%`} />
            <Skeleton width={`${35 + (i % 2) * 15}%`} height={12} />
          </div>
        </div>
      ))}
    </div>
  );
}

type BannerTone = 'info' | 'warning' | 'danger' | 'success';
const BANNER_ICONS = { info: Info, warning: AlertTriangle, danger: AlertCircle, success: CheckCircle2 } as const;

/** An inline, persistent message inside a page or card. */
export function Banner({
  tone = 'info',
  title,
  children,
  action,
}: {
  tone?: BannerTone;
  title?: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
}) {
  const Icon = BANNER_ICONS[tone];
  return (
    <div className={`${styles.banner} ${styles[tone]}`} role={tone === 'danger' ? 'alert' : undefined}>
      <Icon size={18} className={styles.bannerIcon} aria-hidden />
      <div className={styles.bannerText}>
        {title && <p className={styles.bannerTitle}>{title}</p>}
        {children && <div className={styles.bannerBody}>{children}</div>}
      </div>
      {action && <div className={styles.bannerAction}>{action}</div>}
    </div>
  );
}
