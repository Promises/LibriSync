import { forwardRef, type AnchorHTMLAttributes, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { Spinner } from './Spinner';
import styles from './Button.module.css';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'warning';
export type ButtonSize = 'sm' | 'md' | 'lg';

type CommonProps = {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
  /** Icon-only button; `label` becomes the accessible name and tooltip. */
  iconOnly?: boolean;
  label?: string;
};

function classes(variant: ButtonVariant, size: ButtonSize, iconOnly: boolean, extra?: string) {
  return [styles.button, styles[variant], styles[size], iconOnly && styles.iconOnly, extra]
    .filter(Boolean)
    .join(' ');
}

type ButtonProps = CommonProps &
  ButtonHTMLAttributes<HTMLButtonElement> & {
    loading?: boolean;
  };

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    variant = 'secondary',
    size = 'md',
    icon,
    iconOnly = false,
    label,
    loading = false,
    disabled,
    className,
    children,
    type = 'button',
    ...rest
  },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      className={classes(variant, size, iconOnly, className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      aria-label={iconOnly ? label : rest['aria-label']}
      title={iconOnly ? label : rest.title}
      {...rest}
    >
      {loading ? <Spinner size={size === 'sm' ? 14 : 16} /> : icon}
      {!iconOnly && children !== undefined && <span>{children}</span>}
    </button>
  );
});

type LinkButtonProps = CommonProps & AnchorHTMLAttributes<HTMLAnchorElement>;

/** An anchor styled as a button, for actions that navigate (external sign-in pages). */
export function LinkButton({
  variant = 'secondary',
  size = 'md',
  icon,
  iconOnly = false,
  label,
  className,
  children,
  ...rest
}: LinkButtonProps) {
  return (
    <a
      className={classes(variant, size, iconOnly, className)}
      aria-label={iconOnly ? label : undefined}
      title={iconOnly ? label : undefined}
      {...rest}
    >
      {icon}
      {!iconOnly && <span>{children}</span>}
    </a>
  );
}
