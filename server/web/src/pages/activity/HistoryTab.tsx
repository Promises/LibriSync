import { useState } from 'react';
import { Link } from 'react-router';
import { History } from 'lucide-react';
import { useAccounts, useHistory } from '../../api/queries';
import { HISTORY_EVENTS, type HistoryEvent } from '../../api/types';
import { EventIcon } from '../../components/EventIcon';
import { EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import { FilterMenu } from '../../components/FilterMenu';
import { Select } from '../../components/Form';
import { Pagination } from '../../components/Navigation';
import { PROVIDER_NAMES } from '../../components/ProviderIcon';
import { formatDateTime, formatRelative } from '../../lib/format';
import { HISTORY_EVENT } from '../../lib/meta';
import t from '../../components/Table.module.css';
import styles from './HistoryTab.module.css';

export function HistoryTab() {
  const [events, setEvents] = useState<HistoryEvent[]>([]);
  const [account, setAccount] = useState('');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const accounts = useAccounts();
  const history = useHistory({
    page,
    pageSize,
    event: events.length ? events : undefined,
    account: account || undefined,
  });

  const accountName = (id: string | null) => (id ? (accounts.data?.find((a) => a.id === id)?.name ?? id) : null);

  return (
    <>
      <div className={styles.toolbar}>
        <FilterMenu
          label="Event"
          options={HISTORY_EVENTS.map((e) => ({ value: e, label: HISTORY_EVENT[e].label }))}
          selected={events}
          onChange={(e) => {
            setEvents(e);
            setPage(1);
          }}
        />
        <Select
          aria-label="Account"
          value={account}
          onChange={(e) => {
            setAccount(e.target.value);
            setPage(1);
          }}
          className={styles.select}
        >
          <option value="">All accounts</option>
          {accounts.data?.map((a) => (
            <option key={a.id} value={a.id}>
              {a.name} ({PROVIDER_NAMES[a.provider]})
            </option>
          ))}
        </Select>
      </div>

      {history.isPending ? (
        <SkeletonRows rows={8} label="Loading history" />
      ) : history.isError ? (
        <ErrorState error={history.error} onRetry={() => void history.refetch()} />
      ) : history.data.items.length === 0 ? (
        <EmptyState icon={<History size={24} />} title="No history yet">
          {events.length || account
            ? 'Nothing matches these filters.'
            : 'Syncs, downloads and licence events will be listed here as they happen.'}
        </EmptyState>
      ) : (
        <>
          <div className={t.wrap}>
            <table className={t.table}>
              <thead>
                <tr>
                  <th>
                    <span className="visually-hidden">Event type</span>
                  </th>
                  <th>Event</th>
                  <th className={t.hideOnPhone}>Account</th>
                  <th>When</th>
                </tr>
              </thead>
              <tbody>
                {history.data.items.map((h) => (
                  <tr key={h.id}>
                    <td className={styles.iconCol}>
                      <EventIcon event={h.event} />
                    </td>
                    <td>
                      <p className={styles.eventLine}>
                        <span className={styles.eventLabel}>{HISTORY_EVENT[h.event].label}</span>
                        {h.title &&
                          (h.bookId ? (
                            <Link to={`/library?book=${encodeURIComponent(h.bookId)}`}>{h.title}</Link>
                          ) : (
                            <span>{h.title}</span>
                          ))}
                      </p>
                      {h.message && <p className={styles.message}>{h.message}</p>}
                    </td>
                    <td className={t.hideOnPhone}>{accountName(h.accountId) ?? '—'}</td>
                    <td className={styles.when}>
                      <time dateTime={h.time} title={formatDateTime(h.time)}>
                        {formatRelative(h.time)}
                      </time>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <Pagination
            page={page}
            pageSize={pageSize}
            total={history.data.total}
            onPage={setPage}
            onPageSize={(s) => {
              setPageSize(s);
              setPage(1);
            }}
          />
        </>
      )}
    </>
  );
}
