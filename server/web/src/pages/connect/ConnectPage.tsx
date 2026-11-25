import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Cable, Pencil, Plus, Trash2 } from 'lucide-react';
import { notificationsApi } from '../../api/endpoints';
import { keys, useNotifications } from '../../api/queries';
import type { Notification, NotificationKind } from '../../api/types';
import { Badge } from '../../components/Badge';
import { Button } from '../../components/Button';
import { ChoiceIcon } from '../../components/ChoiceList';
import { useConfirm } from '../../components/Confirm';
import { EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import { Switch } from '../../components/Form';
import { PageHeader } from '../../components/Navigation';
import { toast } from '../../components/Toast';
import { formatDateTime, formatRelative } from '../../lib/format';
import { NOTIFICATION_EVENT_LABEL } from '../../lib/meta';
import { KIND_META, KindPicker } from './kinds';
import { NotificationModal } from './NotificationModal';
import styles from './ConnectPage.module.css';

type Editing = { mode: 'pick' } | { mode: 'create'; kind: NotificationKind } | { mode: 'edit'; notification: Notification };

function eventsSummary(n: Notification): string {
  if (n.kind === 'audiobookshelf') return 'Scans the library after each download';
  if (!n.events || n.events.length === 0) return 'No events selected';
  if (n.events.length > 3) return `${n.events.length} events`;
  return n.events.map((e) => NOTIFICATION_EVENT_LABEL[e]).join(', ');
}

export function ConnectPage() {
  const list = useNotifications();
  const client = useQueryClient();
  const confirm = useConfirm();
  const [editing, setEditing] = useState<Editing | null>(null);

  const toggle = useMutation({
    mutationFn: (n: Notification) => notificationsApi.update(n.id, { ...n, enabled: !n.enabled }),
    meta: { errorTitle: 'Could not update the connection' },
    onSuccess: (n) => client.setQueryData<Notification[]>(keys.notifications, (l) => l?.map((x) => (x.id === n.id ? n : x))),
  });
  const remove = useMutation({
    mutationFn: (n: Notification) => notificationsApi.remove(n.id),
    meta: { errorTitle: 'Could not delete the connection' },
    onSuccess: (_r, n) => {
      client.setQueryData<Notification[]>(keys.notifications, (l) => l?.filter((x) => x.id !== n.id));
      toast.success('Connection deleted', n.name);
    },
  });

  const onDelete = async (n: Notification) => {
    const ok = await confirm({
      title: `Delete ${n.name}?`,
      message: <p>LibriSync stops sending to this {KIND_META[n.kind].label} connection.</p>,
      confirmLabel: 'Delete',
      tone: 'danger',
    });
    if (ok) remove.mutate(n);
  };

  const addButton = (
    <Button variant="primary" icon={<Plus size={16} />} onClick={() => setEditing({ mode: 'pick' })}>
      Add connection
    </Button>
  );

  return (
    <>
      <PageHeader
        title="Connect"
        subtitle="Tell other apps when books arrive: webhooks, Apprise (Discord, Telegram, ntfy, email…) and Audiobookshelf."
        actions={list.data && list.data.length > 0 ? addButton : undefined}
      />

      {list.isPending ? (
        <SkeletonRows rows={3} label="Loading connections" />
      ) : list.isError ? (
        <ErrorState error={list.error} onRetry={() => void list.refetch()} />
      ) : list.data.length === 0 ? (
        <EmptyState icon={<Cable size={24} />} title="No connections yet" action={addButton}>
          Connect Audiobookshelf to scan your library as soon as a book is downloaded, or send notifications through a
          webhook or Apprise.
        </EmptyState>
      ) : (
        <ul className={styles.list}>
          {list.data.map((n) => {
            const meta = KIND_META[n.kind];
            return (
              <li key={n.id} className={styles.item}>
                <ChoiceIcon>
                  <meta.Icon size={20} aria-hidden />
                </ChoiceIcon>
                <div className={styles.text}>
                  <p className={styles.name}>
                    {n.name}
                    <Badge tone="neutral">{meta.label}</Badge>
                  </p>
                  <p className={styles.sub}>{eventsSummary(n)}</p>
                  {n.lastResult && (
                    <p className={n.lastResult.ok ? styles.ok : styles.bad} title={formatDateTime(n.lastResult.time)}>
                      {n.lastResult.ok ? 'Last delivery succeeded' : 'Last delivery failed'} {formatRelative(n.lastResult.time)}
                      {n.lastResult.message ? `: ${n.lastResult.message}` : ''}
                    </p>
                  )}
                </div>
                <div className={styles.actions}>
                  <Switch
                    aria-label={`${n.name} enabled`}
                    checked={n.enabled}
                    disabled={toggle.isPending}
                    onChange={() => toggle.mutate(n)}
                  />
                  <Button size="sm" variant="ghost" iconOnly label={`Edit ${n.name}`} icon={<Pencil size={16} />} onClick={() => setEditing({ mode: 'edit', notification: n })} />
                  <Button size="sm" variant="ghost" iconOnly label={`Delete ${n.name}`} icon={<Trash2 size={16} />} onClick={() => void onDelete(n)} />
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <KindPicker
        open={editing?.mode === 'pick'}
        onClose={() => setEditing(null)}
        onPick={(kind) => setEditing({ mode: 'create', kind })}
      />
      {editing && editing.mode !== 'pick' && (
        <NotificationModal
          key={editing.mode === 'edit' ? editing.notification.id : editing.kind}
          initial={editing.mode === 'edit' ? editing.notification : null}
          kind={editing.mode === 'edit' ? editing.notification.kind : editing.kind}
          onClose={() => setEditing(null)}
        />
      )}
    </>
  );
}
