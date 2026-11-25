import type { BookView, QueueItem } from '../../api/types';
import { Cover } from '../../components/Cover';
import { ProviderIcon } from '../../components/ProviderIcon';
import { BookStatusBadge } from '../../components/StatusBadge';
import t from '../../components/Table.module.css';
import { formatDate, formatLength, joinNames } from '../../lib/format';
import styles from './BookTable.module.css';

export function BookTable({
  books,
  queueByBook,
  selected,
  onToggle,
  onToggleAll,
  allSelected,
  onOpen,
}: {
  books: BookView[];
  queueByBook: Map<string, QueueItem>;
  selected: Set<string>;
  onToggle: (id: string) => void;
  onToggleAll: () => void;
  allSelected: boolean;
  onOpen: (book: BookView) => void;
}) {
  return (
    <div className={t.wrap}>
      <table className={`${t.table} ${t.hoverable}`}>
        <thead>
          <tr>
            <th className={t.checkCol}>
              <input
                type="checkbox"
                checked={allSelected}
                onChange={onToggleAll}
                aria-label="Select all books on this page"
              />
            </th>
            <th className={styles.coverCol}>
              <span className="visually-hidden">Cover</span>
            </th>
            <th>Title</th>
            <th className={t.hideOnPhone}>Author</th>
            <th className={t.hideOnPhone}>Length</th>
            <th className={t.hideBelowWide}>Store</th>
            <th className={t.hideBelowWide}>Added</th>
            <th>Status</th>
          </tr>
        </thead>
        <tbody>
          {books.map((book) => {
            const isSelected = selected.has(book.id);
            return (
              <tr key={book.id} className={isSelected ? t.selected : undefined}>
                <td className={t.checkCol}>
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onChange={() => onToggle(book.id)}
                    aria-label={`Select ${book.title}`}
                  />
                </td>
                <td className={styles.coverCol}>
                  <Cover bookId={book.id} hasCover={book.coverUrl !== null} title={book.title} size="thumb" />
                </td>
                <td className={styles.titleCell}>
                  <button type="button" className={t.rowButton} onClick={() => onOpen(book)}>
                    {book.title}
                  </button>
                  {book.series && (
                    <span className={t.sub}>
                      {book.series}
                      {book.seriesSequence ? ` #${book.seriesSequence}` : ''}
                    </span>
                  )}
                  <span className={`${t.sub} ${t.mobileOnly}`}>{joinNames(book.authors, 2)}</span>
                </td>
                <td className={t.hideOnPhone}>{joinNames(book.authors, 2)}</td>
                <td className={`${t.hideOnPhone} ${t.nowrap}`}>{formatLength(book.lengthMinutes)}</td>
                <td className={t.hideBelowWide}>
                  <ProviderIcon provider={book.provider} size="sm" />
                </td>
                <td className={`${t.hideBelowWide} ${t.nowrap}`}>{formatDate(book.addedAt)}</td>
                <td>
                  <BookStatusBadge status={book.status} queueItem={queueByBook.get(book.id)} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
