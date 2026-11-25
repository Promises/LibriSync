import { Link } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { KeyRound, RefreshCw, Snowflake, Trash2 } from 'lucide-react';
import { accountsApi } from '../../api/endpoints';
import { keys, useProviders } from '../../api/queries';
import type { AccountPatch, AccountView } from '../../api/types';
import { Badge } from '../../components/Badge';
import { Button } from '../../components/Button';
import { useConfirm } from '../../components/Confirm';
import { Banner } from '../../components/Feedback';
import { Switch } from '../../components/Form';
import { Spinner } from '../../components/Spinner';
import { Meter } from '../../components/Progress';
import { ProviderIcon, providerName } from '../../components/ProviderIcon';
import { toast } from '../../components/Toast';
import { useNow } from '../../hooks/useNow';
import { formatDateTime, formatDuration, formatNumber, formatRelative } from '../../lib/format';
import styles from './AccountCard.module.css';

function SyncSummary({ account, now }: { account: AccountView; now: number }) {
  const r = account.lastSyncResult;
  if (!account.lastSync) return <span className={styles.muted}>Not synced yet</span>;
  return (
    <span title={formatDateTime(account.lastSync)}>
      {formatRelative(account.lastSync, now)}
      {r && (
        <>
          {' · '}
          {r.ok ? (
            <span className={styles.ok}>
              {r.added > 0 ? `${formatNumber(r.added)} new` : 'no new books'}
              {r.updated > 0 ? `, ${formatNumber(r.updated)} updated` : ''}
            </span>
          ) : (
            <span className={styles.bad}>failed</span>
          )}
        </>
      )}
    </span>
  );
}

export function AccountCard({ account, onReauth }: { account: AccountView; onReauth: (account: AccountView) => void }) {
  const client = useQueryClient();
  const confirm = useConfirm();
  const providers = useProviders();
  const now = useNow();

  const locale = account.locale
    ? (providers.data?.find((p) => p.id === account.provider)?.locales.find((l) => l.code === account.locale)?.name ??
      account.locale.toUpperCase())
    : null;

  const setAccount = (a: AccountView) =>
    client.setQueryData<AccountView[]>(keys.accounts, (list) => list?.map((x) => (x.id === a.id ? a : x)));

  const patch = useMutation({
    mutationFn: (body: AccountPatch) => accountsApi.patch(account.id, body),
    meta: { errorTitle: 'Could not update the account' },
    onSuccess: setAccount,
  });
  const sync = useMutation({
    mutationFn: () => accountsApi.sync(account.id),
    meta: { errorTitle: 'Could not start the sync' },
    onSuccess: () => toast.info('Sync started', account.name),
  });
  const clearCooldown = useMutation({
    mutationFn: () => accountsApi.clearCooldown(account.id),
    meta: { errorTitle: 'Could not clear the cooldown' },
    onSuccess: (a) => {
      setAccount(a);
      toast.success('Cooldown cleared', 'Licence requests for this account resume now.');
    },
  });
  const remove = useMutation({
    mutationFn: (deregister: boolean) => accountsApi.remove(account.id, deregister),
    meta: { errorTitle: 'Could not remove the account' },
    onSuccess: () => {
      client.setQueryData<AccountView[]>(keys.accounts, (list) => list?.filter((x) => x.id !== account.id));
      void client.invalidateQueries({ queryKey: keys.books });
      void client.invalidateQueries({ queryKey: keys.queue });
      toast.success('Account removed', account.name);
    },
  });

  const cooldownMs = account.cooldownUntil ? new Date(account.cooldownUntil).getTime() - now : 0;
  const coolingDown = cooldownMs > 0;
  const isAudible = account.provider === 'audible';

  const onClearCooldown = async () => {
    const ok = await confirm({
      title: 'Clear the cooldown?',
      message: (
        <>
          <p>
            Audible refused licences for this account{account.cooldownReason ? ` (${account.cooldownReason})` : ''}.
            LibriSync is waiting {formatDuration(cooldownMs / 1000)} more before asking again.
          </p>
          <p>
            <strong>Requesting licences while Audible is throttling the account is believed to extend the throttle.</strong>{' '}
            Clear it only if you think the throttle has already lifted.
          </p>
        </>
      ),
      confirmLabel: 'Clear cooldown',
      tone: 'warning',
    });
    if (ok) clearCooldown.mutate();
  };

  const onRemove = async () => {
    const result = await confirm({
      title: `Remove ${account.name}?`,
      message: (
        <p>
          LibriSync stops syncing and downloading from this account. Books already downloaded stay in the library
          folder.
        </p>
      ),
      confirmLabel: 'Remove account',
      tone: 'danger',
      checkbox: isAudible
        ? { label: 'Also deregister this device from the Amazon account', defaultChecked: true }
        : undefined,
    });
    if (result) remove.mutate(isAudible && result.checked);
  };

  return (
    <article className={styles.card} aria-labelledby={`account-${account.id}`}>
      <header className={styles.header}>
        <ProviderIcon provider={account.provider} size="lg" decorative />
        <div className={styles.identity}>
          <h2 id={`account-${account.id}`} className={styles.name}>
            {account.name}
          </h2>
          <p className={styles.muted}>
            {providerName(account.provider)}
            {locale ? ` · ${locale}` : ''}
          </p>
        </div>
        {account.syncing ? (
          <Badge tone="accent" icon={<Spinner size={10} />}>
            Syncing
          </Badge>
        ) : account.authStatus === 'expired' ? (
          <Badge tone="danger">Sign-in expired</Badge>
        ) : !account.enabled ? (
          <Badge tone="neutral">Paused</Badge>
        ) : coolingDown ? (
          <Badge tone="warning" icon={<Snowflake />}>
            Cooldown
          </Badge>
        ) : null}
      </header>

      <div className={styles.body}>
        {account.authStatus === 'expired' && (
          <Banner
            tone="danger"
            title="Sign-in expired"
            action={
              <Button size="sm" variant="primary" icon={<KeyRound size={14} />} onClick={() => onReauth(account)}>
                Sign in again
              </Button>
            }
          >
            The store no longer accepts this account's credentials. Signing in again keeps its settings, history and
            downloads.
          </Banner>
        )}

        {coolingDown && (
          <Banner
            tone="warning"
            title={`Licence cooldown: ${formatDuration(cooldownMs / 1000)} left`}
            action={
              <Button size="sm" variant="ghost" onClick={() => void onClearCooldown()} loading={clearCooldown.isPending}>
                Clear cooldown
              </Button>
            }
          >
            Audible is throttling licence requests
            {account.cooldownReason ? ` (${account.cooldownReason})` : ''}. Downloads for this account resume at{' '}
            {formatDateTime(account.cooldownUntil)}. Syncing continues as normal.
          </Banner>
        )}

        {!coolingDown && account.lastSyncResult && !account.lastSyncResult.ok && (
          <Banner tone="danger" title="Last sync failed">
            {account.lastSyncResult.message ?? 'No details were reported.'}
          </Banner>
        )}

        <dl className={styles.stats}>
          <div>
            <dt>Books</dt>
            <dd>
              <Link to={`/library?account=${encodeURIComponent(account.id)}`}>{formatNumber(account.bookCount)}</Link>
            </dd>
          </div>
          <div>
            <dt>Downloaded</dt>
            <dd>
              {formatNumber(account.downloadedCount)}
              {account.bookCount > 0 && (
                <span className={styles.muted}> ({Math.round((account.downloadedCount / account.bookCount) * 100)}%)</span>
              )}
            </dd>
          </div>
          <div>
            <dt>Last sync</dt>
            <dd>
              <SyncSummary account={account} now={now} />
            </dd>
          </div>
          <div>
            <dt>Next sync</dt>
            <dd title={formatDateTime(account.nextSync)}>
              {account.syncing
                ? 'Syncing now'
                : account.enabled
                  ? account.nextSync
                    ? formatRelative(account.nextSync, now)
                    : '—'
                  : 'Paused'}
            </dd>
          </div>
        </dl>

        {account.dailyLicenceCap !== null && (
          <div className={styles.licences}>
            <div className={styles.licencesLabel}>
              <span>Licences in the last 24 h</span>
              <span className={styles.licencesCount}>
                {formatNumber(account.licencesLast24h)} / {formatNumber(account.dailyLicenceCap)}
              </span>
            </div>
            <Meter
              used={account.licencesLast24h}
              max={account.dailyLicenceCap}
              label={`${account.licencesLast24h} of ${account.dailyLicenceCap} licences used in the last 24 hours`}
            />
          </div>
        )}

        <div className={styles.toggles}>
          <Switch
            label="Scheduled sync"
            description="Include this account in the regular library sync."
            checked={account.enabled}
            disabled={patch.isPending}
            onChange={(enabled) => patch.mutate({ enabled })}
          />
          <Switch
            label="Download new purchases"
            description="Queue books bought after the first sync as soon as they appear."
            checked={account.autoDownloadNew}
            disabled={patch.isPending}
            onChange={(autoDownloadNew) => patch.mutate({ autoDownloadNew })}
          />
          <Switch
            label="Back-fill old library"
            description={
              isAudible
                ? 'Slowly download books owned before the first sync, within the licence budget.'
                : 'Download books owned before the first sync in the background.'
            }
            checked={account.backfill}
            disabled={patch.isPending}
            onChange={(backfill) => patch.mutate({ backfill })}
          />
        </div>
      </div>

      <footer className={styles.footer}>
        <Button variant="ghost" icon={<Trash2 size={16} />} onClick={() => void onRemove()} loading={remove.isPending}>
          Remove
        </Button>
        <div className={styles.footerActions}>
          {isAudible && account.authStatus !== 'expired' && (
            <Button variant="ghost" icon={<KeyRound size={16} />} onClick={() => onReauth(account)}>
              Sign in again
            </Button>
          )}
          <Button
            icon={<RefreshCw size={16} />}
            onClick={() => sync.mutate()}
            loading={sync.isPending || account.syncing}
          >
            {account.syncing ? 'Syncing…' : 'Sync now'}
          </Button>
        </div>
      </footer>
    </article>
  );
}
