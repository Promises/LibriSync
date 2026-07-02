/**
 * Demo Mode predicates and an in-memory replica of the `getBooksWithFilters`
 * query behaviour used by the Audible screens. Pure and self-contained so the
 * demo library can be served entirely from memory (no SQLite).
 */

import type { Account, Book } from '../../../modules/expo-rust-bridge';
import { DEMO_ACCOUNT_ID, DEMO_ASIN_PREFIX, type DemoBook } from './demoData';

/** True when an account id belongs to the demo account (`demo:*`). */
export function isDemoAccountId(id?: string | null): boolean {
  return !!id && id.startsWith('demo:');
}

export function isDemoAccount(account?: Account | null): boolean {
  return isDemoAccountId(account?.account_id);
}

/** True for any book that belongs to the demo dataset (`demo_<id>` asin). */
export function isDemoBook(book?: Book | null): boolean {
  return !!book?.audible_product_id?.startsWith(DEMO_ASIN_PREFIX);
}

/** Strip the demo asin prefix to recover the real LibriVox project id. */
export function librivoxIdFromAsin(asin: string): string {
  return asin.replace(DEMO_ASIN_PREFIX, '');
}

export { DEMO_ACCOUNT_ID };

export interface FilterSortParams {
  offset: number;
  limit: number;
  searchQuery?: string | null;
  series?: string | null;
  category?: string | null;
  sortField?: string | null;
  sortDirection?: string | null;
}

function matchesSearch(book: DemoBook, query: string): boolean {
  const haystack = [
    book.title,
    ...(book.authors || []),
    ...(book.narrators || []),
    book.series_name || '',
  ]
    .join(' ')
    .toLowerCase();
  return haystack.includes(query);
}

function compareBooks(a: DemoBook, b: DemoBook, field: string): number {
  switch (field) {
    case 'length':
      return (a.duration_seconds || 0) - (b.duration_seconds || 0);
    case 'release_date':
      return (a.release_date || '').localeCompare(b.release_date || '');
    case 'date_added':
      return (a.created_at || '').localeCompare(b.created_at || '');
    case 'series': {
      const seriesCmp = (a.series_name || '').localeCompare(b.series_name || '');
      if (seriesCmp !== 0) return seriesCmp;
      return (a.series_sequence || 0) - (b.series_sequence || 0);
    }
    case 'title':
    case 'downloaded': // grouping by download state is irrelevant for the demo
    default:
      return a.title.localeCompare(b.title);
  }
}

/**
 * Replicates the subset of `getBooksWithFilters` the Audible UI relies on:
 * search, series/category filters, sort (field + direction), and pagination.
 */
export function filterSortPaginate(
  source: DemoBook[],
  params: FilterSortParams
): { books: Book[]; total_count: number } {
  const query = (params.searchQuery || '').trim().toLowerCase();

  let filtered = source.filter(book => {
    if (query && !matchesSearch(book, query)) return false;
    if (params.series && book.series_name !== params.series) return false;
    if (params.category && book.category !== params.category) return false;
    return true;
  });

  const field = params.sortField || 'title';
  const direction = params.sortDirection === 'desc' ? -1 : 1;
  filtered = [...filtered].sort((a, b) => compareBooks(a, b, field) * direction);

  const total_count = filtered.length;
  const page = filtered.slice(params.offset, params.offset + params.limit);
  return { books: page, total_count };
}
