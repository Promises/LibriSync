import { useEffect, useId, useRef, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import styles from './FilterMenu.module.css';

/**
 * A button that opens a checklist, for multi-value filters (status, history event).
 * Closes on Escape and outside click; the checkboxes are ordinary form controls.
 */
export function FilterMenu<T extends string>({
  label,
  options,
  selected,
  onChange,
}: {
  label: string;
  options: { value: T; label: string }[];
  selected: T[];
  onChange: (selected: T[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const panelId = useId();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false);
        buttonRef.current?.focus();
      }
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const toggle = (value: T) =>
    onChange(selected.includes(value) ? selected.filter((v) => v !== value) : [...selected, value]);

  const summary = selected.length === 0 ? 'All' : selected.length === 1 ? (options.find((o) => o.value === selected[0])?.label ?? selected[0]) : `${selected.length} selected`;

  return (
    <div className={styles.menu} ref={ref}>
      <button
        ref={buttonRef}
        type="button"
        className={selected.length ? `${styles.trigger} ${styles.triggerActive}` : styles.trigger}
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((o) => !o)}
      >
        <span className={styles.triggerLabel}>{label}:</span>
        <span className={styles.summary}>{summary}</span>
        <ChevronDown size={14} aria-hidden />
      </button>
      {open && (
        <div className={styles.panel} id={panelId} role="group" aria-label={label}>
          {options.map((o) => (
            <label key={o.value} className={styles.option}>
              <input type="checkbox" checked={selected.includes(o.value)} onChange={() => toggle(o.value)} />
              <span>{o.label}</span>
            </label>
          ))}
          {selected.length > 0 && (
            <button type="button" className={styles.clear} onClick={() => onChange([])}>
              Clear
            </button>
          )}
        </div>
      )}
    </div>
  );
}
