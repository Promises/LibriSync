import type { ReactNode } from 'react';
import { NavLink } from 'react-router';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { formatNumber } from '../lib/format';
import { Button } from './Button';
import { Select } from './Form';
import styles from './Navigation.module.css';

/** Route-backed tabs (Activity → Queue / History, System → Status / Tasks / Logs). */
export function TabNav({ tabs, label }: { tabs: { to: string; label: ReactNode }[]; label: string }) {
  return (
    <nav className={styles.tabs} aria-label={label}>
      {tabs.map((t) => (
        <NavLink
          key={t.to}
          to={t.to}
          end
          className={({ isActive }) => (isActive ? `${styles.tab} ${styles.tabActive}` : styles.tab)}
        >
          {t.label}
        </NavLink>
      ))}
    </nav>
  );
}

/** Joined toggle buttons, one pressed at a time (view toggle, sort direction). */
export function Segmented<T extends string>({
  value,
  onChange,
  options,
  label,
}: {
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string; icon?: ReactNode }[];
  label: string;
}) {
  return (
    <div className={styles.segmented} role="group" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          aria-pressed={value === o.value}
          className={value === o.value ? `${styles.segment} ${styles.segmentActive}` : styles.segment}
          onClick={() => onChange(o.value)}
          title={o.icon ? o.label : undefined}
          aria-label={o.icon ? o.label : undefined}
        >
          {o.icon ?? o.label}
        </button>
      ))}
    </div>
  );
}

const PAGE_SIZES = [25, 50, 100, 200];

export function Pagination({
  page,
  pageSize,
  total,
  onPage,
  onPageSize,
}: {
  page: number;
  pageSize: number;
  total: number;
  onPage: (page: number) => void;
  onPageSize?: (size: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / pageSize));
  const from = total === 0 ? 0 : (page - 1) * pageSize + 1;
  const to = Math.min(total, page * pageSize);
  return (
    <nav className={styles.pagination} aria-label="Pagination">
      <p className={styles.range}>
        {formatNumber(from)}–{formatNumber(to)} of {formatNumber(total)}
      </p>
      <div className={styles.pageControls}>
        {onPageSize && (
          <Select
            aria-label="Items per page"
            value={pageSize}
            onChange={(e) => onPageSize(Number(e.target.value))}
            className={styles.pageSize}
          >
            {PAGE_SIZES.map((s) => (
              <option key={s} value={s}>
                {s} / page
              </option>
            ))}
          </Select>
        )}
        <Button
          size="sm"
          iconOnly
          label="Previous page"
          icon={<ChevronLeft size={16} />}
          disabled={page <= 1}
          onClick={() => onPage(page - 1)}
        />
        <span className={styles.pageOf} aria-live="polite">
          Page {page} of {pages}
        </span>
        <Button
          size="sm"
          iconOnly
          label="Next page"
          icon={<ChevronRight size={16} />}
          disabled={page >= pages}
          onClick={() => onPage(page + 1)}
        />
      </div>
    </nav>
  );
}

export function PageHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className={styles.pageHeader}>
      <div className={styles.pageHeading}>
        <h1 className={styles.pageTitle}>{title}</h1>
        {subtitle && <p className={styles.pageSubtitle}>{subtitle}</p>}
      </div>
      {actions && <div className={styles.pageActions}>{actions}</div>}
    </header>
  );
}
