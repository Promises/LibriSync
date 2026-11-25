import { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import { ArrowDownWideNarrow, ArrowUpNarrowWide, Download, LayoutGrid, Library, Rows3, Search, X } from 'lucide-react';
import { useAccounts, useBooks, useQueue } from '../../api/queries';
import type { BookSort, BookStatus, BookView, ProviderId, QueueItem, SortDir } from '../../api/types';
import { Button } from '../../components/Button';
import { EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import { FilterMenu } from '../../components/FilterMenu';
import { Select, TextInput } from '../../components/Form';
import { PageHeader, Pagination, Segmented } from '../../components/Navigation';
import { PROVIDER_NAMES } from '../../components/ProviderIcon';
import { useDebouncedValue } from '../../hooks/useNow';
import { useStoredChoice } from '../../hooks/useLocalStorage';
import { formatNumber } from '../../lib/format';
import { BOOK_STATUS, BOOK_STATUS_ORDER } from '../../lib/meta';
import { BookDrawer } from './BookDrawer';
import { BookGrid } from './BookGrid';
import { BookTable } from './BookTable';
import { useBulkDownload } from './useBulkDownload';
import styles from './LibraryPage.module.css';

const SORTS: { value: BookSort; label: string }[] = [
  { value: 'added', label: 'Date added' },
  { value: 'purchased', label: 'Purchase date' },
  { value: 'released', label: 'Release date' },
  { value: 'title', label: 'Title' },
  { value: 'author', label: 'Author' },
];

const VIEWS = ['grid', 'table'] as const;
const PROVIDERS = Object.keys(PROVIDER_NAMES) as ProviderId[];

function parseStatus(raw: string | null): BookStatus[] {
  if (!raw) return [];
  return raw.split(',').filter((s): s is BookStatus => s in BOOK_STATUS);
}

/** Filters, sort and paging live in the URL so a view can be linked, bookmarked and navigated back to. */
function useLibraryParams() {
  const [params, setParams] = useSearchParams();
  const get = (k: string) => params.get(k);
  const state = {
    search: get('q') ?? '',
    provider: (PROVIDERS as string[]).includes(get('provider') ?? '') ? (get('provider') as ProviderId) : undefined,
    account: get('account') ?? undefined,
    status: parseStatus(get('status')),
    sort: (SORTS.some((s) => s.value === get('sort')) ? get('sort') : 'added') as BookSort,
    dir: (get('dir') === 'asc' ? 'asc' : 'desc') as SortDir,
    page: Math.max(1, Number(get('page')) || 1),
    pageSize: [25, 50, 100, 200].includes(Number(get('pageSize'))) ? Number(get('pageSize')) : 50,
    book: get('book'),
  };
  const update = (changes: Record<string, string | number | null | undefined>, resetPage = true) => {
    setParams(
      (prev) => {
        const next = new URLSearchParams(prev);
        for (const [k, v] of Object.entries(changes)) {
          if (v === null || v === undefined || v === '') next.delete(k);
          else next.set(k, String(v));
        }
        if (resetPage && !('page' in changes)) next.delete('page');
        return next;
      },
      { replace: !('book' in changes) },
    );
  };
  return [state, update] as const;
}

export function LibraryPage() {
  const [p, update] = useLibraryParams();
  const [view, setView] = useStoredChoice('librisync.library.view', VIEWS, 'grid');
  const [searchInput, setSearchInput] = useState(p.search);
  const debouncedSearch = useDebouncedValue(searchInput, 300);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (debouncedSearch !== p.search) update({ q: debouncedSearch });
    // Only the debounced input drives the URL.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [debouncedSearch]);

  const query = {
    search: p.search || undefined,
    provider: p.provider,
    account: p.account,
    status: p.status.length ? p.status : undefined,
    sort: p.sort,
    dir: p.dir,
    page: p.page,
    pageSize: p.pageSize,
  };
  const books = useBooks(query);
  const accounts = useAccounts();
  const queue = useQueue();

  const queueByBook = useMemo(() => {
    const map = new Map<string, QueueItem>();
    queue.data?.forEach((q) => map.set(q.bookId, q));
    return map;
  }, [queue.data]);

  // A new filter is a new result set; a selection across it would be confusing.
  const filterKey = JSON.stringify([p.search, p.provider, p.account, p.status]);
  useEffect(() => setSelected(new Set()), [filterKey]);

  const bulk = useBulkDownload(() => setSelected(new Set()));

  const toggle = (id: string) =>
    setSelected((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const items = books.data?.items ?? [];
  const allOnPage = items.length > 0 && items.every((b) => selected.has(b.id));
  const toggleAll = () =>
    setSelected((s) => {
      const next = new Set(s);
      if (allOnPage) items.forEach((b) => next.delete(b.id));
      else items.forEach((b) => next.add(b.id));
      return next;
    });

  const filtersActive = Boolean(p.search || p.provider || p.account || p.status.length);
  const openBook = (b: BookView) => update({ book: b.id }, false);

  return (
    <>
      <PageHeader
        title="Library"
        subtitle={books.data ? `${formatNumber(books.data.total)} ${filtersActive ? 'matching books' : 'books'}` : undefined}
      />

      <div className={styles.toolbar}>
        <div className={styles.search}>
          <Search size={16} className={styles.searchIcon} aria-hidden />
          <TextInput
            type="search"
            placeholder="Search title, author, narrator, series"
            aria-label="Search the library"
            value={searchInput}
            onChange={(e) => setSearchInput(e.target.value)}
          />
        </div>
        <Select
          aria-label="Store"
          value={p.provider ?? ''}
          onChange={(e) => update({ provider: e.target.value })}
          className={styles.compactSelect}
        >
          <option value="">All stores</option>
          {PROVIDERS.map((id) => (
            <option key={id} value={id}>
              {PROVIDER_NAMES[id]}
            </option>
          ))}
        </Select>
        <Select
          aria-label="Account"
          value={p.account ?? ''}
          onChange={(e) => update({ account: e.target.value })}
          className={styles.compactSelect}
        >
          <option value="">All accounts</option>
          {accounts.data?.map((a) => (
            <option key={a.id} value={a.id}>
              {a.name} ({PROVIDER_NAMES[a.provider]})
            </option>
          ))}
        </Select>
        <FilterMenu
          label="Status"
          options={BOOK_STATUS_ORDER.map((s) => ({ value: s, label: BOOK_STATUS[s].label }))}
          selected={p.status}
          onChange={(s) => update({ status: s.join(',') })}
        />
        <div className={styles.sort}>
          <Select
            aria-label="Sort by"
            value={p.sort}
            onChange={(e) => update({ sort: e.target.value })}
            className={styles.compactSelect}
          >
            {SORTS.map((s) => (
              <option key={s.value} value={s.value}>
                {s.label}
              </option>
            ))}
          </Select>
          <Segmented
            label="Sort direction"
            value={p.dir}
            onChange={(dir) => update({ dir })}
            options={[
              { value: 'asc', label: 'Ascending', icon: <ArrowUpNarrowWide size={16} /> },
              { value: 'desc', label: 'Descending', icon: <ArrowDownWideNarrow size={16} /> },
            ]}
          />
        </div>
        <Segmented
          label="View"
          value={view}
          onChange={setView}
          options={[
            { value: 'grid', label: 'Poster grid', icon: <LayoutGrid size={16} /> },
            { value: 'table', label: 'Table', icon: <Rows3 size={16} /> },
          ]}
        />
      </div>

      {selected.size > 0 && (
        <div className={styles.selectionBar} role="region" aria-label="Selection">
          <span>
            <strong>{formatNumber(selected.size)}</strong> selected
          </span>
          <div className={styles.selectionActions}>
            <Button variant="ghost" size="sm" icon={<X size={14} />} onClick={() => setSelected(new Set())}>
              Clear
            </Button>
            <Button
              variant="primary"
              size="sm"
              icon={<Download size={14} />}
              loading={bulk.isPending}
              onClick={() => void bulk.run([...selected])}
            >
              Download selected
            </Button>
          </div>
        </div>
      )}

      {books.isPending ? (
        <SkeletonRows rows={8} label="Loading books" />
      ) : books.isError ? (
        <ErrorState error={books.error} onRetry={() => void books.refetch()} />
      ) : items.length === 0 ? (
        filtersActive ? (
          <EmptyState
            icon={<Search size={24} />}
            title="No books match"
            action={
              <Button
                onClick={() => {
                  setSearchInput('');
                  update({ q: null, provider: null, account: null, status: null });
                }}
              >
                Clear filters
              </Button>
            }
          >
            Try a different search or remove some filters.
          </EmptyState>
        ) : (
          <EmptyState
            icon={<Library size={24} />}
            title="Your library is empty"
            action={<Link to="/accounts">Go to Accounts</Link>}
          >
            Add an Audible, Libro.fm or Chirp account and LibriSync will sync the books you own.
          </EmptyState>
        )
      ) : view === 'grid' ? (
        <BookGrid books={items} queueByBook={queueByBook} selected={selected} onToggle={toggle} onOpen={openBook} />
      ) : (
        <BookTable
          books={items}
          queueByBook={queueByBook}
          selected={selected}
          onToggle={toggle}
          onToggleAll={toggleAll}
          allSelected={allOnPage}
          onOpen={openBook}
        />
      )}

      {books.data && books.data.total > 0 && (
        <Pagination
          page={p.page}
          pageSize={p.pageSize}
          total={books.data.total}
          onPage={(page) => update({ page })}
          onPageSize={(pageSize) => update({ pageSize })}
        />
      )}

      <BookDrawer bookId={p.book} onClose={() => update({ book: null }, false)} />
    </>
  );
}
