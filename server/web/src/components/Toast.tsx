import { useSyncExternalStore } from 'react';
import { AlertTriangle, CheckCircle2, Info, X, XCircle } from 'lucide-react';
import styles from './Toast.module.css';

type ToastTone = 'success' | 'error' | 'info' | 'warning';
type ToastItem = { id: number; tone: ToastTone; title: string; message?: string };

// A module-level store so anything (mutation callbacks, the query cache) can raise a
// toast without threading a context through.
let items: ToastItem[] = [];
let nextId = 1;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

function dismiss(id: number) {
  items = items.filter((t) => t.id !== id);
  emit();
}

function push(tone: ToastTone, title: string, message?: string) {
  const id = nextId++;
  // Identical toasts in a burst (a failing poll) collapse into one.
  if (items.some((t) => t.title === title && t.message === message)) return;
  items = [...items.slice(-4), { id, tone, title, message }];
  emit();
  setTimeout(() => dismiss(id), tone === 'error' ? 8000 : 4500);
}

export const toast = {
  success: (title: string, message?: string) => push('success', title, message),
  error: (title: string, message?: string) => push('error', title, message),
  info: (title: string, message?: string) => push('info', title, message),
  warning: (title: string, message?: string) => push('warning', title, message),
};

const ICONS = {
  success: CheckCircle2,
  error: XCircle,
  info: Info,
  warning: AlertTriangle,
} as const;

export function ToastViewport() {
  const list = useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => items,
  );
  return (
    <section className={styles.viewport} aria-label="Notifications" aria-live="polite">
      {list.map((t) => {
        const Icon = ICONS[t.tone];
        return (
          <div key={t.id} className={`${styles.toast} ${styles[t.tone]}`} role={t.tone === 'error' ? 'alert' : 'status'}>
            <Icon size={18} className={styles.icon} aria-hidden />
            <div className={styles.text}>
              <p className={styles.title}>{t.title}</p>
              {t.message && <p className={styles.message}>{t.message}</p>}
            </div>
            <button type="button" className={styles.close} onClick={() => dismiss(t.id)} aria-label="Dismiss">
              <X size={16} />
            </button>
          </div>
        );
      })}
    </section>
  );
}
