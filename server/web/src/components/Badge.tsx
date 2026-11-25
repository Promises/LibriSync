import type { ReactNode } from 'react';
import styles from './Badge.module.css';

export type Tone = 'success' | 'warning' | 'danger' | 'info' | 'neutral' | 'special' | 'accent';

export function Badge({
  tone = 'neutral',
  icon,
  children,
  title,
}: {
  tone?: Tone;
  icon?: ReactNode;
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className={`${styles.badge} ${styles[tone]}`} title={title}>
      {icon}
      <span>{children}</span>
    </span>
  );
}

/** A small count bubble, e.g. on a navigation item. */
export function CountBubble({ tone = 'accent', count, label }: { tone?: Tone; count: number; label: string }) {
  return (
    <span className={`${styles.bubble} ${styles[tone]}`} aria-label={label} title={label}>
      {count > 99 ? '99+' : count}
    </span>
  );
}
