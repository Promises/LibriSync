import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Eye, EyeOff, RefreshCw } from 'lucide-react';
import { settingsApi } from '../../api/endpoints';
import { keys, useBooks } from '../../api/queries';
import type { Settings } from '../../api/types';
import { Button } from '../../components/Button';
import { Card } from '../../components/Card';
import { useConfirm } from '../../components/Confirm';
import { CopyButton } from '../../components/CopyButton';
import { Banner } from '../../components/Feedback';
import { FieldRow, FormField, NumberInput, Select, Switch, TextInput } from '../../components/Form';
import { toast } from '../../components/Toast';
import {
  EXAMPLE_SERIES_BOOK,
  EXAMPLE_STANDALONE_BOOK,
  outputFileExample,
  renderFolderTemplate,
  TEMPLATE_TOKENS,
  type TemplateBook,
} from '../../lib/folderTemplate';
import type { FieldErrors } from './validate';
import styles from './SettingsSections.module.css';

export type SetField = <S extends keyof Settings, K extends keyof Settings[S]>(
  section: S,
  key: K,
  value: Settings[S][K],
) => void;

type SectionProps = { draft: Settings; set: SetField; errors: FieldErrors };

export const SECTIONS = [
  { id: 'media', title: 'Media management' },
  { id: 'scheduling', title: 'Scheduling' },
  { id: 'backfill', title: 'Back-fill' },
  { id: 'audible', title: 'Audible pacing' },
  { id: 'downloads', title: 'Downloads' },
  { id: 'general', title: 'General' },
] as const;

const title = (id: (typeof SECTIONS)[number]['id']) => SECTIONS.find((s) => s.id === id)?.title ?? id;

// ---- Media management ---------------------------------------------------------------

/** A real series book and a real standalone from the library, when there are any. */
function usePreviewBooks(): { series: TemplateBook; standalone: TemplateBook; examples: boolean } {
  const books = useBooks({ page: 1, pageSize: 100, sort: 'added', dir: 'desc' });
  const items = books.data?.items ?? [];
  const series = items.find((b) => b.series);
  const standalone = items.find((b) => !b.series);
  return {
    series: series ?? EXAMPLE_SERIES_BOOK,
    standalone: standalone ?? EXAMPLE_STANDALONE_BOOK,
    examples: !series || !standalone,
  };
}

function TemplatePreview({ template, book, format }: { template: string; book: TemplateBook; format: Settings['media']['format'] }) {
  const result = renderFolderTemplate(template, book);
  return (
    <div className={styles.preview}>
      <code className={styles.previewPath}>
        <span className={styles.previewRoot}>…/</span>
        {result.path || <em>(empty)</em>}/<span className={styles.previewFile}>{outputFileExample(book, format)}</span>
      </code>
      {result.unknownTokens.length > 0 && (
        <p className={styles.previewWarning}>Unknown token{result.unknownTokens.length > 1 ? 's' : ''}: {result.unknownTokens.join(', ')}</p>
      )}
    </div>
  );
}

export function MediaSection({ draft, set, errors }: SectionProps) {
  const preview = usePreviewBooks();
  const m = draft.media;
  return (
    <Card id="media" title={title('media')} description="Where books are written and how folders are named.">
      <div className={styles.stack}>
        <FormField label="Library folder" hint="Set by the LIBRISYNC_LIBRARY_DIR environment variable / the container's volume mount.">
          {(p) => <TextInput {...p} value={m.libraryDir} readOnly />}
        </FormField>

        <FormField
          label="Folder for books in a series"
          error={errors['media.folderTemplate']}
          hint={
            <>
              Example{preview.examples ? ' (sample book)' : ` (${preview.series.title})`}:
            </>
          }
        >
          {(p) => (
            <TextInput
              {...p}
              className={styles.mono}
              spellCheck={false}
              value={m.folderTemplate}
              onChange={(e) => set('media', 'folderTemplate', e.target.value)}
            />
          )}
        </FormField>
        <TemplatePreview template={m.folderTemplate} book={preview.series} format={m.format} />

        <FormField
          label="Folder for standalone books"
          error={errors['media.noSeriesFolderTemplate']}
          hint={
            <>
              Example{preview.examples ? ' (sample book)' : ` (${preview.standalone.title})`}:
            </>
          }
        >
          {(p) => (
            <TextInput
              {...p}
              className={styles.mono}
              spellCheck={false}
              value={m.noSeriesFolderTemplate}
              onChange={(e) => set('media', 'noSeriesFolderTemplate', e.target.value)}
            />
          )}
        </FormField>
        <TemplatePreview template={m.noSeriesFolderTemplate} book={preview.standalone} format={m.format} />

        <details className={styles.tokens}>
          <summary>Available tokens</summary>
          <dl>
            {TEMPLATE_TOKENS.map((t) => (
              <div key={t.token}>
                <dt>
                  <code>{t.token}</code>
                </dt>
                <dd>{t.description}</dd>
              </div>
            ))}
          </dl>
          <p className={styles.hint}>
            Use <code>/</code> to make sub-folders. Characters not allowed in file names are removed, and each folder
            name is cut to 120 bytes. Keep one folder per book: Audiobookshelf treats a folder as a book.
          </p>
        </details>

        <FieldRow>
          <FormField
            label="Output format"
            error={errors['media.format']}
            hint={m.format === 'm4b' ? 'One chaptered M4B file per book.' : 'One MP3 per chapter, named “NN - Chapter Title.mp3”.'}
          >
            {(p) => (
              <Select {...p} value={m.format} onChange={(e) => set('media', 'format', e.target.value as Settings['media']['format'])}>
                <option value="m4b">M4B (single file, chapters)</option>
                <option value="mp3">MP3 per chapter</option>
              </Select>
            )}
          </FormField>
        </FieldRow>

        <Switch
          label="Write sidecar files"
          description="cover.jpg, desc.txt, reader.txt and metadata.opf next to the audio, for Audiobookshelf and other players."
          checked={m.writeSidecars}
          onChange={(v) => set('media', 'writeSidecars', v)}
        />
      </div>
    </Card>
  );
}

// ---- Scheduling ---------------------------------------------------------------------

export function SchedulingSection({ draft, set, errors }: SectionProps) {
  return (
    <Card id="scheduling" title={title('scheduling')} description="How often each account's library is checked for new purchases.">
      <div className={styles.stack}>
        <FormField
          label="Sync every (hours)"
          error={errors['sync.intervalHours']}
          hint="1 to 168. Each sync is offset by up to ±30 minutes so accounts do not all hit the store at once."
        >
          {(p) => (
            <NumberInput {...p} min={1} max={168} value={draft.sync.intervalHours} onChange={(v) => set('sync', 'intervalHours', v)} />
          )}
        </FormField>
        <Switch
          label="Download new purchases automatically"
          description="Default for accounts; each account can override it on the Accounts page."
          checked={draft.sync.autoDownloadNew}
          onChange={(v) => set('sync', 'autoDownloadNew', v)}
        />
      </div>
    </Card>
  );
}

// ---- Back-fill ----------------------------------------------------------------------

export function BackfillSection({ draft, set, errors }: SectionProps) {
  const b = draft.backfill;
  const windowed = b.windowStart !== null || b.windowEnd !== null;
  return (
    <Card
      id="backfill"
      title={title('backfill')}
      description="Back-fill downloads the books you owned before adding the account, a few at a time, so the whole library arrives without tripping Audible's throttle."
    >
      <div className={styles.stack}>
        <Switch label="Back-fill the existing library" checked={b.enabled} onChange={(v) => set('backfill', 'enabled', v)} />
        <FormField label="Order" error={errors['backfill.order']}>
          {(p) => (
            <Select {...p} value={b.order} disabled={!b.enabled} onChange={(e) => set('backfill', 'order', e.target.value as Settings['backfill']['order'])}>
              <option value="newest_first">Newest purchases first</option>
              <option value="oldest_first">Oldest purchases first</option>
            </Select>
          )}
        </FormField>
        <Switch
          label="Only during a time window"
          description="Run back-fill only between these local times, e.g. overnight. New purchases and manual downloads are not limited."
          checked={windowed}
          disabled={!b.enabled}
          onChange={(on) => {
            set('backfill', 'windowStart', on ? '01:00' : null);
            set('backfill', 'windowEnd', on ? '07:00' : null);
          }}
        />
        {windowed && (
          <FieldRow>
            <FormField label="From" error={errors['backfill.windowStart']}>
              {(p) => (
                <TextInput {...p} type="time" value={b.windowStart ?? ''} disabled={!b.enabled} onChange={(e) => set('backfill', 'windowStart', e.target.value || null)} />
              )}
            </FormField>
            <FormField label="Until" error={errors['backfill.windowEnd']} hint="An end before the start spans midnight.">
              {(p) => (
                <TextInput {...p} type="time" value={b.windowEnd ?? ''} disabled={!b.enabled} onChange={(e) => set('backfill', 'windowEnd', e.target.value || null)} />
              )}
            </FormField>
          </FieldRow>
        )}
      </div>
    </Card>
  );
}

// ---- Audible pacing -----------------------------------------------------------------

export function AudibleSection({ draft, set, errors }: SectionProps) {
  const a = draft.audible;
  return (
    <Card id="audible" title={title('audible')}>
      <div className={styles.stack}>
        <Banner tone="info" title="Nobody knows Audible's real limits">
          Audible stops granting download licences to accounts that request too many (“CustomerThrottled”), and does
          not say where the line is. These defaults are deliberately conservative. Raising them makes the back-fill
          faster and the throttle more likely; a throttle pauses all Audible downloads for that account for a day or
          more.
        </Banner>
        <FormField
          label="Licences per account per 24 hours"
          error={errors['audible.dailyLicenceCap']}
          hint="Every licence request counts, granted or not. 1 to 200; default 20."
        >
          {(p) => (
            <NumberInput {...p} min={1} max={200} value={a.dailyLicenceCap} onChange={(v) => set('audible', 'dailyLicenceCap', v)} />
          )}
        </FormField>
        <FieldRow>
          <FormField label="Wait between requests, at least (minutes)" error={errors['audible.spacingMinMinutes']}>
            {(p) => (
              <NumberInput {...p} min={0} value={a.spacingMinMinutes} onChange={(v) => set('audible', 'spacingMinMinutes', v)} />
            )}
          </FormField>
          <FormField label="…and at most (minutes)" error={errors['audible.spacingMaxMinutes']}>
            {(p) => (
              <NumberInput {...p} min={0} value={a.spacingMaxMinutes} onChange={(v) => set('audible', 'spacingMaxMinutes', v)} />
            )}
          </FormField>
        </FieldRow>
        <p className={styles.hint}>
          A random wait in this range separates back-fill licence requests for the same account (default 20–60). New
          purchases and manual downloads skip the wait but still count towards the daily limit.
        </p>
        <FormField
          label="Cooldown after a throttle (hours)"
          error={errors['audible.throttleCooldownHours']}
          hint="How long to stop asking after Audible throttles an account. Doubles on each consecutive throttle, up to 7 days. Default 24."
        >
          {(p) => (
            <NumberInput
              {...p}
              min={1}
              max={168}
              value={a.throttleCooldownHours}
              onChange={(v) => set('audible', 'throttleCooldownHours', v)}
            />
          )}
        </FormField>
        <Switch
          label="Download Audible Plus catalogue titles"
          description="Plus titles can only be downloaded while you have an active Audible Plus membership. Off: they're left out of back-fill and new-purchase downloads; you can still download one manually."
          checked={a.includePlusCatalog}
          onChange={(v) => set('audible', 'includePlusCatalog', v)}
        />
      </div>
    </Card>
  );
}

// ---- Downloads ----------------------------------------------------------------------

export function DownloadsSection({ draft, set, errors }: SectionProps) {
  return (
    <Card id="downloads" title={title('downloads')}>
      <FormField
        label="Simultaneous downloads"
        error={errors['downloads.concurrency']}
        hint="1 to 4. Audible licence requests stay one at a time per account regardless."
      >
        {(p) => (
          <NumberInput {...p} min={1} max={4} value={draft.downloads.concurrency} onChange={(v) => set('downloads', 'concurrency', v)} />
        )}
      </FormField>
    </Card>
  );
}

// ---- General ------------------------------------------------------------------------

function ApiKeyField({ apiKey }: { apiKey: string }) {
  const [revealed, setRevealed] = useState(false);
  const client = useQueryClient();
  const confirm = useConfirm();
  const regenerate = useMutation({
    mutationFn: settingsApi.regenerateApiKey,
    meta: { errorTitle: 'Could not regenerate the API key' },
    onSuccess: ({ apiKey: next }) => {
      client.setQueryData<Settings>(keys.settings, (s) => (s ? { ...s, general: { ...s.general, apiKey: next } } : s));
      setRevealed(true);
      toast.success('New API key generated', 'Update any scripts or apps that use the old one.');
    },
  });
  const onRegenerate = async () => {
    const ok = await confirm({
      title: 'Regenerate the API key?',
      message: <p>The current key stops working immediately. Anything that uses it must be updated.</p>,
      confirmLabel: 'Regenerate',
      tone: 'danger',
    });
    if (ok) regenerate.mutate();
  };
  return (
    <FormField label="API key" hint="Send as the X-Api-Key header or ?apikey= query parameter.">
      {(p) => (
        <div className={styles.apiKey}>
          <TextInput {...p} className={styles.mono} readOnly value={revealed ? apiKey : '•'.repeat(Math.min(32, apiKey.length || 32))} />
          <Button
            iconOnly
            label={revealed ? 'Hide API key' : 'Show API key'}
            icon={revealed ? <EyeOff size={16} /> : <Eye size={16} />}
            onClick={() => setRevealed((r) => !r)}
          />
          <CopyButton text={apiKey} iconOnly label="Copy API key" size="md" />
          <Button iconOnly label="Regenerate API key" icon={<RefreshCw size={16} />} onClick={() => void onRegenerate()} loading={regenerate.isPending} />
        </div>
      )}
    </FormField>
  );
}

export function GeneralSection({ draft, set, errors }: SectionProps) {
  const g = draft.general;
  return (
    <Card id="general" title={title('general')}>
      <div className={styles.stack}>
        <FormField label="Instance name" error={errors['general.instanceName']} hint="Shown in notifications, to tell several servers apart.">
          {(p) => <TextInput {...p} value={g.instanceName} onChange={(e) => set('general', 'instanceName', e.target.value)} />}
        </FormField>
        <ApiKeyField apiKey={g.apiKey} />
        <FormField label="Log level" error={errors['general.logLevel']} hint="Debug is verbose; use it while chasing a problem.">
          {(p) => (
            <Select {...p} value={g.logLevel} onChange={(e) => set('general', 'logLevel', e.target.value as Settings['general']['logLevel'])}>
              <option value="debug">Debug</option>
              <option value="info">Info</option>
              <option value="warn">Warnings only</option>
            </Select>
          )}
        </FormField>
      </div>
    </Card>
  );
}
