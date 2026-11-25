import { Ban, Download, RotateCcw, Snowflake, Trash2, Undo2 } from 'lucide-react';
import { useAccounts, useBook, useQueue } from '../../api/queries';
import type { BookDetail, HistoryItem } from '../../api/types';
import { Badge } from '../../components/Badge';
import { Button } from '../../components/Button';
import { CopyButton } from '../../components/CopyButton';
import { Cover } from '../../components/Cover';
import { EventIcon } from '../../components/EventIcon';
import { Banner, ErrorState, Skeleton } from '../../components/Feedback';
import { Modal } from '../../components/Modal';
import { ProgressBar } from '../../components/Progress';
import { ProviderIcon } from '../../components/ProviderIcon';
import { BookStatusBadge } from '../../components/StatusBadge';
import { useNow } from '../../hooks/useNow';
import {
  formatDate,
  formatDateTime,
  formatDuration,
  formatLength,
  formatPercent,
  formatRelative,
  htmlToParagraphs,
  joinNames,
} from '../../lib/format';
import { HISTORY_EVENT, QUEUE_STAGE } from '../../lib/meta';
import { coolingAccount, useBookActions } from './useBookActions';
import styles from './BookDrawer.module.css';

export function BookDrawer({ bookId, onClose }: { bookId: string | null; onClose: () => void }) {
  const book = useBook(bookId);
  const title = book.data?.title ?? (book.isPending ? 'Loading…' : 'Book');
  return (
    <Modal
      open={bookId !== null}
      onClose={onClose}
      title={title}
      placement="side"
      footer={book.data ? <BookActions book={book.data} /> : undefined}
    >
      {book.isPending ? (
        <DrawerSkeleton />
      ) : book.isError ? (
        <ErrorState error={book.error} onRetry={() => void book.refetch()} />
      ) : (
        <BookDetails book={book.data} />
      )}
    </Modal>
  );
}

function DrawerSkeleton() {
  return (
    <div className={styles.top} role="status" aria-label="Loading book">
      <Skeleton width={160} height={160} />
      <div className={styles.facts}>
        <Skeleton width="70%" height={20} />
        <Skeleton width="50%" />
        <Skeleton width="40%" />
      </div>
    </div>
  );
}

function BookDetails({ book }: { book: BookDetail }) {
  const accounts = useAccounts();
  const queue = useQueue();
  const now = useNow();
  const queueItem = queue.data?.find((q) => q.bookId === book.id);
  const accountNames = book.accounts.map((id) => accounts.data?.find((a) => a.id === id)?.name ?? id);
  const cooling = coolingAccount(book, accounts.data, now);
  const description = htmlToParagraphs(book.description);

  return (
    <div className={styles.details}>
      <div className={styles.top}>
        <Cover bookId={book.id} hasCover={book.coverUrl !== null} title={book.title} size="large" />
        <div className={styles.facts}>
          {book.subtitle && <p className={styles.subtitle}>{book.subtitle}</p>}
          <p className={styles.byline}>
            by <strong>{joinNames(book.authors, 5)}</strong>
          </p>
          {book.narrators.length > 0 && (
            <p className={styles.muted}>Narrated by {joinNames(book.narrators, 5)}</p>
          )}
          {book.series && (
            <p className={styles.muted}>
              {book.series}
              {book.seriesSequence ? `, book ${book.seriesSequence}` : ''}
            </p>
          )}
          <div className={styles.badges}>
            <BookStatusBadge status={book.status} queueItem={queueItem} />
            {book.format && <Badge tone="neutral">{book.format.toUpperCase()}</Badge>}
          </div>
        </div>
      </div>

      {queueItem && (queueItem.state === 'running' || queueItem.state === 'planning') && (
        <div className={styles.progress}>
          <p className={styles.muted}>
            {queueItem.stage ? QUEUE_STAGE[queueItem.stage] : 'Requesting licence'}
            {queueItem.parts && queueItem.parts > 1 ? ` · part ${queueItem.part ?? 1} of ${queueItem.parts}` : ''}
            {queueItem.progress !== null ? ` · ${formatPercent(queueItem.progress)}` : ''}
          </p>
          <ProgressBar value={queueItem.progress} label={`Progress of ${book.title}`} />
        </div>
      )}
      {queueItem?.state === 'waiting' && queueItem.waitingReason && (
        <Banner tone="info" title="Waiting">
          {queueItem.waitingReason}
        </Banner>
      )}
      {cooling && book.status !== 'downloaded' && (
        <Banner tone="warning" title={`${cooling.name} is cooling down`}>
          Audible throttled licence requests for this account. They resume in{' '}
          {formatDuration((new Date(cooling.cooldownUntil as string).getTime() - now) / 1000)}.
        </Banner>
      )}
      {book.lastError && (
        <Banner tone="danger" title="Last attempt failed">
          {book.lastError}
          {book.backoffUntil && new Date(book.backoffUntil).getTime() > now && (
            <> Next try {formatRelative(book.backoffUntil, now)}.</>
          )}
        </Banner>
      )}

      <dl className={styles.meta}>
        <div>
          <dt>Store</dt>
          <dd>
            <ProviderIcon provider={book.provider} size="sm" withLabel />
          </dd>
        </div>
        <div>
          <dt>Length</dt>
          <dd>{formatLength(book.lengthMinutes)}</dd>
        </div>
        <div>
          <dt>Released</dt>
          <dd>{formatDate(book.releaseDate)}</dd>
        </div>
        <div>
          <dt>Purchased</dt>
          <dd>{formatDate(book.purchaseDate)}</dd>
        </div>
        <div>
          <dt>Added</dt>
          <dd>{formatDate(book.addedAt)}</dd>
        </div>
        <div>
          <dt>{book.accounts.length === 1 ? 'Account' : 'Accounts'}</dt>
          <dd>{accountNames.join(', ') || '—'}</dd>
        </div>
        <div>
          <dt>Product id</dt>
          <dd className={styles.mono}>{book.id}</dd>
        </div>
        {book.downloadedAt && (
          <div>
            <dt>Downloaded</dt>
            <dd>{formatDateTime(book.downloadedAt)}</dd>
          </div>
        )}
      </dl>

      {book.outputPath && (
        <section className={styles.section} aria-labelledby="book-output">
          <h3 id="book-output" className={styles.sectionTitle}>
            Location
          </h3>
          <div className={styles.path}>
            <code>{book.outputPath}</code>
            <CopyButton text={book.outputPath} iconOnly label="Copy path" />
          </div>
          <p className={styles.hint}>Relative to the library folder.</p>
        </section>
      )}

      {description.length > 0 && (
        <section className={styles.section} aria-labelledby="book-desc">
          <h3 id="book-desc" className={styles.sectionTitle}>
            Description
          </h3>
          <div className={styles.description}>
            {description.map((p, i) => (
              <p key={i}>{p}</p>
            ))}
          </div>
        </section>
      )}

      <section className={styles.section} aria-labelledby="book-history">
        <h3 id="book-history" className={styles.sectionTitle}>
          History
        </h3>
        {book.history.length === 0 ? (
          <p className={styles.hint}>Nothing has happened to this book yet.</p>
        ) : (
          <ol className={styles.history}>
            {book.history.map((h) => (
              <HistoryRow key={h.id} item={h} />
            ))}
          </ol>
        )}
      </section>
    </div>
  );
}

function HistoryRow({ item }: { item: HistoryItem }) {
  const meta = HISTORY_EVENT[item.event];
  return (
    <li className={styles.historyItem}>
      <EventIcon event={item.event} />
      <div className={styles.historyText}>
        <p>
          <strong>{meta.label}</strong> · <time dateTime={item.time}>{formatDateTime(item.time)}</time>
        </p>
        {item.message && <p className={styles.muted}>{item.message}</p>}
      </div>
    </li>
  );
}

function BookActions({ book }: { book: BookDetail }) {
  const actions = useBookActions();
  const accounts = useAccounts();
  const now = useNow();
  const cooling = coolingAccount(book, accounts.data, now);
  const inFlight = book.status === 'queued' || book.status === 'downloading';
  const canDownload = !inFlight && book.status !== 'downloaded';

  return (
    <div className={styles.actions}>
      {book.status === 'skipped' ? (
        <Button icon={<Undo2 size={16} />} onClick={() => actions.unskip(book)} disabled={actions.busy}>
          Unskip
        </Button>
      ) : (
        !inFlight &&
        book.status !== 'downloaded' && (
          <Button variant="ghost" icon={<Ban size={16} />} onClick={() => actions.skip(book)} disabled={actions.busy}>
            Skip
          </Button>
        )
      )}
      {(book.status === 'downloaded' || book.status === 'failed') && (
        <Button variant="ghost" icon={<Trash2 size={16} />} onClick={() => void actions.forget(book)} disabled={actions.busy}>
          Forget
        </Button>
      )}
      {book.status === 'downloaded' && (
        <Button icon={<RotateCcw size={16} />} onClick={() => void actions.download(book)} disabled={actions.busy}>
          Download again
        </Button>
      )}
      {canDownload && cooling && (
        <Button
          variant="warning"
          icon={<Snowflake size={16} />}
          onClick={() => void actions.forceDownload(book, cooling.cooldownUntil)}
          disabled={actions.busy}
        >
          Download anyway
        </Button>
      )}
      {canDownload && !cooling && (
        <Button variant="primary" icon={<Download size={16} />} onClick={() => void actions.download(book)} disabled={actions.busy}>
          Download
        </Button>
      )}
    </div>
  );
}
