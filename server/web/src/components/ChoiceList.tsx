import type { ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import styles from './ChoiceList.module.css';

export type Choice<K extends string> = {
  key: K;
  icon: ReactNode;
  title: ReactNode;
  description?: ReactNode;
};

/** A vertical list of large, fully clickable options (provider picker, connection kind). */
export function ChoiceList<K extends string>({ choices, onPick }: { choices: Choice<K>[]; onPick: (key: K) => void }) {
  return (
    <ul className={styles.choices}>
      {choices.map((c) => (
        <li key={c.key}>
          <button type="button" className={styles.choice} onClick={() => onPick(c.key)}>
            {c.icon}
            <span className={styles.text}>
              <span className={styles.title}>{c.title}</span>
              {c.description && <span className={styles.description}>{c.description}</span>}
            </span>
            <ChevronRight size={18} aria-hidden className={styles.chevron} />
          </button>
        </li>
      ))}
    </ul>
  );
}

/** The tinted square used as a choice's icon when it is a plain glyph. */
export function ChoiceIcon({ children }: { children: ReactNode }) {
  return <span className={styles.icon}>{children}</span>;
}
