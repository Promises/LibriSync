import { useState } from 'react';
import { BookAudio } from 'lucide-react';
import { booksApi } from '../api/endpoints';
import styles from './Cover.module.css';

/**
 * A book cover through the server's caching proxy, lazy-loaded. Falls back to a
 * placeholder when the book has no cover or the image fails.
 */
export function Cover({
  bookId,
  hasCover,
  title,
  size = 'fill',
}: {
  bookId: string;
  hasCover: boolean;
  title: string;
  size?: 'fill' | 'thumb' | 'large';
}) {
  const [failed, setFailed] = useState(false);
  const className = `${styles.cover} ${styles[size]}`;
  if (!hasCover || failed) {
    return (
      <div className={`${className} ${styles.placeholder}`} aria-hidden>
        <BookAudio size={size === 'thumb' ? 18 : 36} />
      </div>
    );
  }
  return (
    <img
      className={className}
      src={booksApi.coverUrl(bookId)}
      alt=""
      title={size === 'thumb' ? title : undefined}
      loading="lazy"
      decoding="async"
      onError={() => setFailed(true)}
    />
  );
}
