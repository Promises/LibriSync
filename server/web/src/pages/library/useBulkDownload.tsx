import { useMutation, useQueryClient } from '@tanstack/react-query';
import { booksApi } from '../../api/endpoints';
import { keys } from '../../api/queries';
import { useConfirm } from '../../components/Confirm';
import { toast } from '../../components/Toast';
import { pluralize } from '../../lib/format';

/** Above this many books, ask first: a big Audible batch is exactly what trips the throttle. */
const CONFIRM_ABOVE = 5;

/** "Download selected" for Library and Wanted. */
export function useBulkDownload(onDone: () => void) {
  const client = useQueryClient();
  const confirm = useConfirm();

  const mutation = useMutation({
    mutationFn: (ids: string[]) => booksApi.downloadMany(ids),
    meta: { errorTitle: 'Could not queue downloads' },
    onSuccess: (res) => {
      onDone();
      if (res.skipped.length === 0) toast.success(`Queued ${pluralize(res.queued, 'book')}`);
      else
        toast.warning(
          `Queued ${pluralize(res.queued, 'book')}, skipped ${res.skipped.length}`,
          res.skipped
            .slice(0, 3)
            .map((s) => s.reason)
            .join('; '),
        );
      void client.invalidateQueries({ queryKey: keys.books });
      void client.invalidateQueries({ queryKey: keys.wanted });
      void client.invalidateQueries({ queryKey: keys.queue });
    },
  });

  const run = async (ids: string[]) => {
    if (ids.length > CONFIRM_ABOVE) {
      const ok = await confirm({
        title: `Download ${pluralize(ids.length, 'book')}?`,
        message: (
          <>
            <p>
              They are queued ahead of back-fill, but Audible books still count against each account's daily licence
              limit and wait for it, so a large batch can take days. Accounts in a cooldown wait until it ends.
            </p>
            <p>To fetch a whole backlog, enable back-fill instead; it does this at a safe pace on its own.</p>
          </>
        ),
        confirmLabel: 'Queue downloads',
        tone: 'primary',
      });
      if (!ok) return;
    }
    mutation.mutate(ids);
  };

  return { run, isPending: mutation.isPending };
}
