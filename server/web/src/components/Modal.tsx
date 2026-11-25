import { useEffect, useId, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { Button } from './Button';
import styles from './Modal.module.css';

/**
 * A modal on the native <dialog> element: the browser provides the focus trap, Escape,
 * inertness of the page behind and the top layer. `placement="side"` makes it a drawer.
 */
export function Modal({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  size = 'md',
  placement = 'center',
  dismissible = true,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  size?: 'sm' | 'md' | 'lg';
  placement?: 'center' | 'side';
  /** False while something irreversible is in flight. */
  dismissible?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descId = useId();

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  if (!open) return null;

  return (
    <dialog
      ref={ref}
      className={[styles.dialog, styles[size], placement === 'side' && styles.side].filter(Boolean).join(' ')}
      aria-labelledby={titleId}
      aria-describedby={description ? descId : undefined}
      onCancel={(e) => {
        e.preventDefault();
        if (dismissible) onClose();
      }}
      onMouseDown={(e) => {
        // A press on the backdrop lands on the dialog element itself.
        if (e.target === e.currentTarget && dismissible) onClose();
      }}
    >
      <div className={styles.frame}>
        <header className={styles.header}>
          <div className={styles.heading}>
            <h2 id={titleId} className={styles.title}>
              {title}
            </h2>
            {description && (
              <p id={descId} className={styles.description}>
                {description}
              </p>
            )}
          </div>
          <Button
            variant="ghost"
            iconOnly
            label="Close"
            icon={<X size={18} />}
            onClick={onClose}
            disabled={!dismissible}
          />
        </header>
        <div className={styles.body}>{children}</div>
        {footer && <footer className={styles.footer}>{footer}</footer>}
      </div>
    </dialog>
  );
}
