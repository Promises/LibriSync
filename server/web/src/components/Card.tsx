import { useId, type ReactNode } from 'react';
import styles from './Card.module.css';

export function Card({
  id,
  title,
  description,
  actions,
  children,
  as: Tag = 'section',
  flush = false,
}: {
  /** Anchor target, e.g. for an in-page table of contents. */
  id?: string;
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  as?: 'section' | 'article' | 'div';
  /** Body without padding, for tables and lists that run edge to edge. */
  flush?: boolean;
}) {
  const titleId = useId();
  return (
    <Tag id={id} className={styles.card} aria-labelledby={title ? titleId : undefined}>
      {(title || actions) && (
        <header className={styles.header}>
          <div className={styles.heading}>
            {title && (
              <h2 className={styles.title} id={titleId}>
                {title}
              </h2>
            )}
            {description && <p className={styles.description}>{description}</p>}
          </div>
          {actions && <div className={styles.actions}>{actions}</div>}
        </header>
      )}
      {children !== undefined && <div className={flush ? styles.bodyFlush : styles.body}>{children}</div>}
    </Tag>
  );
}
