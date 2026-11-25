import { useMutation, useQueryClient } from '@tanstack/react-query';
import { errorMessage, isApiError } from '../../api/client';
import { booksApi } from '../../api/endpoints';
import { keys } from '../../api/queries';
import type { AccountView, BookView } from '../../api/types';
import { useConfirm } from '../../components/Confirm';
import { toast } from '../../components/Toast';
import { formatDateTime } from '../../lib/format';

/** The owning account that is in a licence cooldown right now, if any. */
export function coolingAccount(book: Pick<BookView, 'accounts'>, accounts: AccountView[] | undefined, now = Date.now()) {
  return accounts?.find(
    (a) => book.accounts.includes(a.id) && a.cooldownUntil !== null && new Date(a.cooldownUntil).getTime() > now,
  );
}

const FORCE_WARNING = (
  <>
    <p>
      Audible refused a recent licence request for this account and LibriSync is pausing licence requests to let
      the throttle clear.
    </p>
    <p>
      <strong>Requesting another licence now is believed to extend the throttle.</strong> Only do this if you
      need this book urgently.
    </p>
  </>
);

/** Download / force / skip / unskip / forget, with the confirmations each needs. */
export function useBookActions() {
  const client = useQueryClient();
  const confirm = useConfirm();

  const refresh = (id: string) => {
    void client.invalidateQueries({ queryKey: keys.books });
    void client.invalidateQueries({ queryKey: keys.wanted });
    void client.invalidateQueries({ queryKey: keys.queue });
    void client.invalidateQueries({ queryKey: keys.book(id) });
  };

  const downloadMutation = useMutation({
    mutationFn: ({ id, force }: { id: string; force: boolean }) => booksApi.download(id, force),
    meta: { silent: true },
  });

  const confirmForce = async (until: string | null, serverMessage?: string) =>
    (await confirm({
      title: 'Download anyway?',
      message: (
        <>
          {until ? <p>This account is cooling down until {formatDateTime(until)}.</p> : serverMessage && <p>{serverMessage}</p>}
          {FORCE_WARNING}
        </>
      ),
      confirmLabel: 'Download anyway',
      tone: 'warning',
    })) !== null;

  const download = async (book: BookView, force = false) => {
    try {
      await downloadMutation.mutateAsync({ id: book.id, force });
      toast.success(force ? 'Forced download queued' : 'Download queued', book.title);
      refresh(book.id);
    } catch (e) {
      if (!force && isApiError(e) && e.error === 'account_cooldown') {
        if (await confirmForce(null, e.message)) await download(book, true);
        return;
      }
      toast.error('Could not queue the download', errorMessage(e));
    }
  };

  const forceDownload = async (book: BookView, cooldownUntil: string | null) => {
    if (await confirmForce(cooldownUntil)) await download(book, true);
  };

  const simple = useMutation({
    mutationFn: ({ id, action }: { id: string; action: 'skip' | 'unskip' | 'forget' }) => booksApi[action](id),
    meta: { errorTitle: 'Could not update the book' },
    onSuccess: (book) => {
      client.setQueryData(keys.book(book.id), (old: object | undefined) => (old ? { ...old, ...book } : old));
      refresh(book.id);
    },
  });

  const skip = (book: BookView) => simple.mutate({ id: book.id, action: 'skip' });
  const unskip = (book: BookView) => simple.mutate({ id: book.id, action: 'unskip' });
  const forget = async (book: BookView) => {
    const ok = await confirm({
      title: 'Forget download record?',
      message: (
        <p>
          LibriSync will treat <strong>{book.title}</strong> as not downloaded. Files already in the library folder are
          left untouched; downloading again writes a fresh copy.
        </p>
      ),
      confirmLabel: 'Forget',
      tone: 'danger',
    });
    if (ok) simple.mutate({ id: book.id, action: 'forget' });
  };

  return {
    download: (book: BookView) => download(book, false),
    forceDownload,
    skip,
    unskip,
    forget,
    busy: downloadMutation.isPending || simple.isPending,
  };
}
