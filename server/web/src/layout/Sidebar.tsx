import type { ComponentType } from 'react';
import { NavLink } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Activity,
  BookMarked,
  CircleDashed,
  Cable,
  LogOut,
  Monitor,
  Moon,
  Server,
  Settings,
  Sun,
  Users,
  X,
  type LucideProps,
} from 'lucide-react';
import { authApi } from '../api/endpoints';
import type { EventStreamStatus } from '../api/events';
import { keys, useAccounts, useQueue, useSystemStatus } from '../api/queries';
import type { AuthState, HealthItem } from '../api/types';
import { CountBubble, type Tone } from '../components/Badge';
import { Button } from '../components/Button';
import { useTheme } from '../hooks/useTheme';
import styles from './Sidebar.module.css';

type NavItem = {
  to: string;
  label: string;
  Icon: ComponentType<LucideProps>;
  badge?: { count: number; tone: Tone; label: string } | null;
};

const THEME_META = {
  system: { Icon: Monitor, label: 'Theme: follow system' },
  light: { Icon: Sun, label: 'Theme: light' },
  dark: { Icon: Moon, label: 'Theme: dark' },
} as const;

const STREAM_LABEL: Record<EventStreamStatus, string> = {
  open: 'Live',
  connecting: 'Connecting…',
  reconnecting: 'Reconnecting…',
};

export function Sidebar({
  id,
  open,
  onClose,
  username,
  streamStatus,
  health,
}: {
  id: string;
  open: boolean;
  onClose: () => void;
  username: string | null;
  streamStatus: EventStreamStatus;
  health: HealthItem[];
}) {
  const client = useQueryClient();
  const queue = useQueue();
  const accounts = useAccounts();
  const status = useSystemStatus();
  const theme = useTheme();

  const logout = useMutation({
    mutationFn: authApi.logout,
    meta: { errorTitle: 'Could not sign out' },
    onSuccess: () => {
      client.removeQueries({ predicate: (q) => q.queryKey[0] !== keys.authState[0] });
      client.setQueryData<AuthState>(keys.authState, (s) => (s ? { ...s, authenticated: false, username: null } : s));
    },
  });

  const queueCount = queue.data?.length ?? 0;
  const failedCount = queue.data?.filter((q) => q.state === 'failed').length ?? 0;
  const accountProblems = (accounts.data ?? []).filter((a) => a.authStatus === 'expired' || a.cooldownUntil).length;
  const healthErrors = health.filter((h) => h.type === 'error').length;
  const healthWarnings = health.filter((h) => h.type === 'warning').length;

  const items: NavItem[] = [
    { to: '/library', label: 'Library', Icon: BookMarked },
    {
      to: '/activity',
      label: 'Activity',
      Icon: Activity,
      badge: queueCount
        ? { count: queueCount, tone: failedCount ? 'danger' : 'accent', label: `${queueCount} in queue${failedCount ? `, ${failedCount} failed` : ''}` }
        : null,
    },
    { to: '/wanted', label: 'Wanted', Icon: CircleDashed },
    {
      to: '/accounts',
      label: 'Accounts',
      Icon: Users,
      badge: accountProblems ? { count: accountProblems, tone: 'warning', label: `${accountProblems} accounts need attention` } : null,
    },
    { to: '/connect', label: 'Connect', Icon: Cable },
    { to: '/settings', label: 'Settings', Icon: Settings },
    {
      to: '/system',
      label: 'System',
      Icon: Server,
      badge:
        healthErrors || healthWarnings
          ? {
              count: healthErrors + healthWarnings,
              tone: healthErrors ? 'danger' : 'warning',
              label: `${healthErrors + healthWarnings} health issues`,
            }
          : null,
    },
  ];

  const ThemeIcon = THEME_META[theme.preference].Icon;

  return (
    <aside id={id} className={open ? `${styles.sidebar} ${styles.open}` : styles.sidebar} aria-label="Main">
      <div className={styles.brand}>
        <img src="/favicon.svg" alt="" width={32} height={32} className={styles.logo} />
        <div className={styles.brandText}>
          <span className={styles.brandName}>LibriSync</span>
          <span className={styles.brandSub}>Server{status.data ? ` · v${status.data.version}` : ''}</span>
        </div>
        <Button
          className={styles.close}
          variant="ghost"
          iconOnly
          label="Close menu"
          icon={<X size={18} />}
          onClick={onClose}
        />
      </div>

      <nav className={styles.nav}>
        <ul>
          {items.map(({ to, label, Icon, badge }) => (
            <li key={to}>
              <NavLink
                to={to}
                className={({ isActive }) => (isActive ? `${styles.link} ${styles.active}` : styles.link)}
              >
                <Icon size={18} aria-hidden className={styles.linkIcon} />
                <span className={styles.linkLabel}>{label}</span>
                {badge && <CountBubble count={badge.count} tone={badge.tone} label={badge.label} />}
              </NavLink>
            </li>
          ))}
        </ul>
      </nav>

      <div className={styles.footer}>
        <p className={styles.stream} data-status={streamStatus}>
          <span className={styles.dot} aria-hidden />
          {STREAM_LABEL[streamStatus]}
        </p>
        <div className={styles.footerRow}>
          <span className={styles.user} title={username ?? undefined}>
            {username ?? 'Signed in'}
          </span>
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            label={`${THEME_META[theme.preference].label} (click to change)`}
            icon={<ThemeIcon size={16} />}
            onClick={theme.cycle}
          />
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            label="Sign out"
            icon={<LogOut size={16} />}
            loading={logout.isPending}
            onClick={() => logout.mutate()}
          />
        </div>
      </div>
    </aside>
  );
}
