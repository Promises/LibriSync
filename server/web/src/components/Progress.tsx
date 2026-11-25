import styles from './Progress.module.css';

type ProgressTone = 'accent' | 'success' | 'warning' | 'danger';

/** A determinate bar for 0..1, or an indeterminate one when `value` is null. */
export function ProgressBar({
  value,
  tone = 'accent',
  label,
  size = 'md',
}: {
  value: number | null;
  tone?: ProgressTone;
  label: string;
  size?: 'sm' | 'md';
}) {
  const clamped = value === null ? null : Math.min(1, Math.max(0, value));
  return (
    <div
      className={`${styles.track} ${styles[size]}`}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={clamped === null ? undefined : Math.round(clamped * 100)}
    >
      <div
        className={clamped === null ? `${styles.fill} ${styles.indeterminate} ${styles[tone]}` : `${styles.fill} ${styles[tone]}`}
        style={clamped === null ? undefined : { width: `${clamped * 100}%` }}
      />
    </div>
  );
}

/** A usage meter (licences used / cap) that warns as it fills. */
export function Meter({ used, max, label }: { used: number; max: number; label: string }) {
  const ratio = max > 0 ? used / max : 0;
  const tone: ProgressTone = ratio >= 1 ? 'danger' : ratio >= 0.75 ? 'warning' : 'accent';
  return <ProgressBar value={Math.min(1, ratio)} tone={tone} label={label} size="sm" />;
}
