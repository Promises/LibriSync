import {
  forwardRef,
  useId,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from 'react';
import styles from './Form.module.css';

export type ControlProps = {
  id: string;
  'aria-describedby'?: string;
  'aria-invalid'?: true;
};

/**
 * Label + control + hint + error, wired together for assistive tech. The control is a
 * render prop so it can be any input while still receiving the generated ids.
 */
export function FormField({
  label,
  hint,
  error,
  children,
  inline = false,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: string | null;
  children: (props: ControlProps) => ReactNode;
  /** Label beside the control (checkbox/switch rows). */
  inline?: boolean;
}) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(' ') || undefined;
  const control = children({
    id,
    'aria-describedby': describedBy,
    'aria-invalid': error ? true : undefined,
  });
  return (
    <div className={inline ? `${styles.field} ${styles.fieldInline}` : styles.field}>
      {inline ? (
        <div className={styles.inlineRow}>
          <label htmlFor={id} className={styles.label}>
            {label}
          </label>
          {control}
        </div>
      ) : (
        <>
          <label htmlFor={id} className={styles.label}>
            {label}
          </label>
          {control}
        </>
      )}
      {hint && (
        <p id={hintId} className={styles.hint}>
          {hint}
        </p>
      )}
      {error && (
        <p id={errorId} className={styles.error} role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

export const TextInput = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  function TextInput({ className, ...rest }, ref) {
    return <input ref={ref} className={[styles.input, className].filter(Boolean).join(' ')} {...rest} />;
  },
);

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select className={[styles.input, styles.select, className].filter(Boolean).join(' ')} {...rest}>
      {children}
    </select>
  );
}

export function Textarea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={[styles.input, styles.textarea, className].filter(Boolean).join(' ')} {...rest} />;
}

export function Checkbox({
  label,
  className,
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'type'> & { label?: ReactNode }) {
  const input = <input type="checkbox" className={[styles.checkbox, className].filter(Boolean).join(' ')} {...rest} />;
  if (!label) return input;
  return (
    <label className={styles.checkboxLabel}>
      {input}
      <span>{label}</span>
    </label>
  );
}

/** A checkbox that looks and announces like a switch. */
export function Switch({
  checked,
  onChange,
  label,
  description,
  disabled,
  id,
  ...aria
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
  id?: string;
  'aria-describedby'?: string;
  'aria-label'?: string;
}) {
  const generated = useId();
  const inputId = id ?? generated;
  const descId = description ? `${inputId}-desc` : undefined;
  const control = (
    <input
      id={inputId}
      type="checkbox"
      role="switch"
      className={styles.switch}
      checked={checked}
      disabled={disabled}
      onChange={(e) => onChange(e.target.checked)}
      aria-describedby={[aria['aria-describedby'], descId].filter(Boolean).join(' ') || undefined}
      aria-label={aria['aria-label']}
    />
  );
  if (!label) return control;
  return (
    <div className={styles.switchRow}>
      <div className={styles.switchText}>
        <label htmlFor={inputId} className={styles.switchLabel}>
          {label}
        </label>
        {description && (
          <p id={descId} className={styles.hint}>
            {description}
          </p>
        )}
      </div>
      {control}
    </div>
  );
}

/** A group of related fields with a heading, used by Settings and Connect forms. */
export function Fieldset({
  legend,
  description,
  children,
}: {
  legend: ReactNode;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <fieldset className={styles.fieldset}>
      <legend className={styles.legend}>{legend}</legend>
      {description && <p className={styles.fieldsetDescription}>{description}</p>}
      <div className={styles.fieldsetBody}>{children}</div>
    </fieldset>
  );
}

/**
 * An integer input whose empty state is NaN rather than 0, so validation can say
 * "required" instead of silently saving a zero.
 */
export function NumberInput({
  value,
  onChange,
  className,
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'type'> & {
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <input
      type="number"
      inputMode="numeric"
      className={[styles.input, styles.number, className].filter(Boolean).join(' ')}
      value={Number.isNaN(value) ? '' : value}
      onChange={(e) => onChange(e.target.value === '' ? Number.NaN : Number(e.target.value))}
      {...rest}
    />
  );
}

/** Side-by-side fields that stack on narrow screens. */
export function FieldRow({ children }: { children: ReactNode }) {
  return <div className={styles.row}>{children}</div>;
}
