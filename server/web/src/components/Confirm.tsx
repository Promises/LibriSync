import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from 'react';
import { AlertTriangle } from 'lucide-react';
import { Button } from './Button';
import { Checkbox } from './Form';
import { Modal } from './Modal';
import styles from './Confirm.module.css';

export type ConfirmOptions = {
  title: string;
  message: ReactNode;
  confirmLabel: string;
  tone?: 'danger' | 'warning' | 'primary';
  /** An extra opt-in shown as a checkbox; its state is returned. */
  checkbox?: { label: ReactNode; defaultChecked?: boolean };
};

export type ConfirmResult = { checked: boolean };

type Pending = ConfirmOptions & { resolve: (r: ConfirmResult | null) => void };

const ConfirmContext = createContext<((o: ConfirmOptions) => Promise<ConfirmResult | null>) | null>(null);

/** `await confirm({...})` resolves to `{checked}` when confirmed, `null` when cancelled. */
export function useConfirm() {
  const ctx = useContext(ConfirmContext);
  if (!ctx) throw new Error('useConfirm needs <ConfirmProvider>');
  return ctx;
}

export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const [checked, setChecked] = useState(false);
  const pendingRef = useRef<Pending | null>(null);

  const confirm = useCallback((options: ConfirmOptions) => {
    return new Promise<ConfirmResult | null>((resolve) => {
      pendingRef.current?.resolve(null);
      const next = { ...options, resolve };
      pendingRef.current = next;
      setChecked(options.checkbox?.defaultChecked ?? false);
      setPending(next);
    });
  }, []);

  const finish = (result: ConfirmResult | null) => {
    pendingRef.current?.resolve(result);
    pendingRef.current = null;
    setPending(null);
  };

  const tone = pending?.tone ?? 'danger';

  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      <Modal
        open={pending !== null}
        onClose={() => finish(null)}
        title={pending?.title ?? ''}
        size="sm"
        footer={
          <>
            <Button variant="secondary" onClick={() => finish(null)}>
              Cancel
            </Button>
            <Button variant={tone} onClick={() => finish({ checked })} autoFocus>
              {pending?.confirmLabel}
            </Button>
          </>
        }
      >
        <div className={styles.body}>
          {tone !== 'primary' && (
            <AlertTriangle
              size={22}
              className={tone === 'danger' ? styles.iconDanger : styles.iconWarning}
              aria-hidden
            />
          )}
          <div className={styles.message}>
            {pending?.message}
            {pending?.checkbox && (
              <div className={styles.checkbox}>
                <Checkbox
                  label={pending.checkbox.label}
                  checked={checked}
                  onChange={(e) => setChecked(e.target.checked)}
                />
              </div>
            )}
          </div>
        </div>
      </Modal>
    </ConfirmContext.Provider>
  );
}
