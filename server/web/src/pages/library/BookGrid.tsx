import type { BookView, QueueItem } from '../../api/types';
import { Cover } from '../../components/Cover';
import { ProgressBar } from '../../components/Progress';
import { ProviderIcon } from '../../components/ProviderIcon';
import { BookStatusBadge } from '../../components/StatusBadge';
import { joinNames } from '../../lib/format';
import styles from './BookGrid.module.css';

export function BookGrid({
  books,
  queueByBook,
  selected,
  onToggle,
  onOpen,
}: {
  books: BookView[];
  queueByBook: Map<string, QueueItem>;
  selected: Set<string>;
  onToggle: (id: string) => void;
  onOpen: (book: BookView) => void;
}) {
  const selecting = selected.size > 0;
  return (
    <ul className={selecting ? `${styles.grid} ${styles.selecting}` : styles.grid}>
      {books.map((book) => {
        const q = queueByBook.get(book.id);
        const isSelected = selected.has(book.id);
        const running = q && (q.state === 'running' || q.state === 'planning');
        return (
          <li key={book.id} className={isSelected ? `${styles.card} ${styles.selected}` : styles.card}>
            <div className={styles.poster}>
              <Cover bookId={book.id} hasCover={book.coverUrl !== null} title={book.title} />
              <label className={styles.check}>
                <input type="checkbox" checked={isSelected} onChange={() => onToggle(book.id)} />
                <span className="visually-hidden">Select {book.title}</span>
              </label>
              <span className={styles.provider}>
                <ProviderIcon provider={book.provider} size="sm" />
              </span>
              <span className={styles.status}>
                <BookStatusBadge status={book.status} queueItem={q} />
              </span>
              {running && (
                <div className={styles.progress}>
                  <ProgressBar value={q.progress} label={`Progress of ${book.title}`} size="sm" />
                </div>
              )}
            </div>
            <button type="button" className={styles.text} onClick={() => onOpen(book)}>
              <span className={styles.title}>{book.title}</span>
              <span className={styles.author}>{joinNames(book.authors, 2)}</span>
              {book.series && (
                <span className={styles.series}>
                  {book.series}
                  {book.seriesSequence ? ` #${book.seriesSequence}` : ''}
                </span>
              )}
            </button>
          </li>
        );
      })}
    </ul>
  );
}
