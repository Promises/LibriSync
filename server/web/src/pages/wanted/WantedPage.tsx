import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import { CheckCircle2, Download, X } from 'lucide-react';
import { useQueue, useSettings, useWanted } from '../../api/queries';
import type { BookView, QueueItem, WantedPage as WantedResponse } from '../../api/types';
import { Button } from '../../components/Button';
import { Cover } from '../../components/Cover';
import { Banner, EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import { PageHeader, Pagination } from '../../components/Navigation';
import { ProviderIcon } from '../../components/ProviderIcon';
import { BookStatusBadge, PlusBadge } from '../../components/StatusBadge';
import t from '../../components/Table.module.css';
import { formatDate, formatLength, formatNumber, joinNames, pluralize } from '../../lib/format';
import { BookDrawer } from '../library/BookDrawer';
import { useBulkDownload } from '../library/useBulkDownload';
import styles from './WantedPage.module.css';

function BackfillEstimate({ data }: { data: WantedResponse }) {
  const settings = useSettings();
  const { perDay, daysRemaining } = data.estimate;
  if (settings.data && !settings.data.backfill.enabled) {
    return (
      <Banner
        tone="info"
        title="Back-fill is off"
        action={<Link to="/settings#backfill">Back-fill settings</Link>}
      >
        These books are only downloaded when you pick them. To fetch the whole backlog, turn back-fill on: it works
        through the library in the background at a pace Audible tolerates.
      </Banner>
    );
  }
  return (
    <Banner tone="info" title={`Back-fill: about ${formatNumber(Math.round(perDay))} per day`}>
      {daysRemaining === null
        ? 'No estimate yet — it appears once back-fill has run for a while.'
        : daysRemaining <= 1
          ? 'The rest of the library should be downloaded within a day.'
          : `About ${formatNumber(Math.ceil(daysRemaining))} days until the whole library is downloaded.`}{' '}
      Audible books are paced by the daily licence cap to avoid Audible's throttle; Libro.fm and Chirp are limited
      only by download concurrency.
    </Banner>
  );
}

export function WantedPage() {
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [params, setParams] = useSearchParams();
  const wanted = useWanted({ page, pageSize });
  const queue = useQueue();

  const queueByBook = useMemo(() => {
    const m = new Map<string, QueueItem>();
    queue.data?.forEach((q) => m.set(q.bookId, q));
    return m;
  }, [queue.data]);

  const bulk = useBulkDownload(() => setSelected(new Set()));

  const items = wanted.data?.items ?? [];
  const allSelected = items.length > 0 && items.every((b) => selected.has(b.id));
  const toggle = (id: string) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const toggleAll = () =>
    setSelected((s) => {
      const n = new Set(s);
      items.forEach((b) => (allSelected ? n.delete(b.id) : n.add(b.id)));
      return n;
    });

  const open = (b: BookView) => setParams({ book: b.id });
  const bookId = params.get('book');

  return (
    <>
      <PageHeader
        title="Wanted"
        subtitle={
          wanted.data
            ? `${pluralize(wanted.data.total, 'book')} you own but have not downloaded yet.`
            : 'Books you own but have not downloaded yet.'
        }
        actions={
          // Picking is for books wanted soon; the backlog as a whole is back-fill's job.
          selected.size > 0 && (
            <Button variant="primary" icon={<Download size={16} />} loading={bulk.isPending} onClick={() => void bulk.run([...selected])}>
              Download {formatNumber(selected.size)} selected
            </Button>
          )
        }
      />

      {wanted.data && wanted.data.total > 0 && (
        <div className={styles.estimate}>
          <BackfillEstimate data={wanted.data} />
        </div>
      )}

      {selected.size > 0 && (
        <p className={styles.selection}>
          {formatNumber(selected.size)} selected
          <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={() => setSelected(new Set())}>
            Clear
          </Button>
        </p>
      )}

      {wanted.isPending ? (
        <SkeletonRows rows={8} label="Loading wanted books" />
      ) : wanted.isError ? (
        <ErrorState error={wanted.error} onRetry={() => void wanted.refetch()} />
      ) : items.length === 0 ? (
        <EmptyState icon={<CheckCircle2 size={24} />} title="Nothing missing">
          Every book in your library is downloaded, queued, skipped or excluded.
        </EmptyState>
      ) : (
        <>
          <div className={t.wrap}>
            <table className={`${t.table} ${t.hoverable}`}>
              <thead>
                <tr>
                  <th className={t.checkCol}>
                    <input type="checkbox" checked={allSelected} onChange={toggleAll} aria-label="Select all on this page" />
                  </th>
                  <th className={styles.coverCol}>
                    <span className="visually-hidden">Cover</span>
                  </th>
                  <th>Title</th>
                  <th className={t.hideOnPhone}>Length</th>
                  <th className={t.hideBelowWide}>Store</th>
                  <th className={t.hideBelowWide}>Purchased</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {items.map((b) => (
                  <tr key={b.id} className={selected.has(b.id) ? t.selected : undefined}>
                    <td className={t.checkCol}>
                      <input
                        type="checkbox"
                        checked={selected.has(b.id)}
                        onChange={() => toggle(b.id)}
                        aria-label={`Select ${b.title}`}
                      />
                    </td>
                    <td className={styles.coverCol}>
                      <Cover bookId={b.id} hasCover={b.coverUrl !== null} title={b.title} size="thumb" />
                    </td>
                    <td>
                      <button type="button" className={t.rowButton} onClick={() => open(b)}>
                        {b.title}
                      </button>
                      <span className={t.sub}>{joinNames(b.authors, 2)}</span>
                    </td>
                    <td className={`${t.hideOnPhone} ${t.nowrap}`}>{formatLength(b.lengthMinutes)}</td>
                    <td className={t.hideBelowWide}>
                      <ProviderIcon provider={b.provider} size="sm" />
                    </td>
                    <td className={`${t.hideBelowWide} ${t.nowrap}`}>{formatDate(b.purchaseDate)}</td>
                    <td>
                      <span className={styles.badges}>
                        <BookStatusBadge status={b.status} queueItem={queueByBook.get(b.id)} />
                        {b.plusCatalog && <PlusBadge />}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <Pagination
            page={page}
            pageSize={pageSize}
            total={wanted.data.total}
            onPage={(p) => {
              setPage(p);
              setSelected(new Set());
            }}
            onPageSize={(s) => {
              setPageSize(s);
              setPage(1);
            }}
          />
        </>
      )}

      <BookDrawer bookId={bookId} onClose={() => setParams({})} />
    </>
  );
}
