import { useEffect, useRef, useState, type FormEvent } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { FlaskConical, Library, Plus, Trash2 } from 'lucide-react';
import { errorMessage } from '../../api/client';
import { notificationsApi } from '../../api/endpoints';
import { keys } from '../../api/queries';
import {
  HISTORY_EVENTS,
  SECRET_MASK,
  type AbsConfig,
  type AbsLibrary,
  type AppriseConfig,
  type HistoryEvent,
  type Notification,
  type NotificationDraft,
  type NotificationKind,
  type NotificationTestResult,
  type WebhookConfig,
} from '../../api/types';
import { Button } from '../../components/Button';
import { Banner } from '../../components/Feedback';
import { Checkbox, FieldRow, Fieldset, FormField, Select, Switch, TextInput, Textarea } from '../../components/Form';
import { Modal } from '../../components/Modal';
import { toast } from '../../components/Toast';
import { HISTORY_EVENT } from '../../lib/meta';
import { KIND_META } from './kinds';
import styles from './NotificationModal.module.css';

const DEFAULT_EVENTS: HistoryEvent[] = ['download', 'downloadFailed', 'licenceThrottled', 'syncFailed', 'authExpired'];

function blankDraft(kind: NotificationKind): NotificationDraft {
  const base = { name: KIND_META[kind].label, enabled: true };
  switch (kind) {
    case 'webhook':
      return { ...base, kind, events: DEFAULT_EVENTS, config: { url: '', method: 'POST', headers: [] } };
    case 'apprise':
      return { ...base, kind, events: DEFAULT_EVENTS, config: { serverUrl: '', configKey: '', statelessUrls: '', tags: '' } };
    case 'audiobookshelf':
      return { ...base, kind, events: null, config: { url: '', apiKey: '', libraryId: '', pathPrefix: '' } };
  }
}

function isHttpUrl(s: string): boolean {
  try {
    const u = new URL(s);
    return u.protocol === 'http:' || u.protocol === 'https:';
  } catch {
    return false;
  }
}

type Errors = Partial<Record<string, string>>;

function validate(d: NotificationDraft): Errors {
  const e: Errors = {};
  if (!d.name.trim()) e.name = 'Give the connection a name.';
  if (d.kind === 'webhook') {
    if (!isHttpUrl(d.config.url)) e.url = 'Enter an http:// or https:// URL.';
    if (d.config.headers.some((h) => !h.key.trim())) e.headers = 'Every header needs a name.';
  }
  if (d.kind === 'apprise') {
    if (!isHttpUrl(d.config.serverUrl)) e.serverUrl = 'Enter the apprise-api server URL.';
    if (!d.config.configKey?.trim() && !d.config.statelessUrls?.trim())
      e.configKey = 'Set a configuration key or stateless URLs.';
  }
  if (d.kind === 'audiobookshelf') {
    if (!isHttpUrl(d.config.url)) e.url = 'Enter the Audiobookshelf URL.';
    if (!d.config.apiKey.trim()) e.apiKey = 'Enter an API key.';
    if (!d.config.libraryId) e.libraryId = 'Pick the library to scan.';
  }
  if (d.events !== null && d.events.length === 0) e.events = 'Pick at least one event.';
  return e;
}

/** Empty optional strings are omitted rather than sent as "". */
function clean(d: NotificationDraft): NotificationDraft {
  const strip = <T extends object>(o: T): T =>
    Object.fromEntries(Object.entries(o).filter(([, v]) => v !== '' && v !== undefined)) as T;
  return { ...d, name: d.name.trim(), config: strip(d.config) } as NotificationDraft;
}

export function NotificationModal({
  initial,
  kind,
  onClose,
}: {
  initial: Notification | null;
  kind: NotificationKind;
  onClose: () => void;
}) {
  const client = useQueryClient();
  const [draft, setDraft] = useState<NotificationDraft>(() => {
    if (!initial) return blankDraft(kind);
    const { lastResult: _unused, ...rest } = initial;
    return rest;
  });
  const [showErrors, setShowErrors] = useState(false);
  const [testResult, setTestResult] = useState<NotificationTestResult | null>(null);
  const errors = showErrors ? validate(draft) : {};
  const meta = KIND_META[kind];
  const resultRef = useRef<HTMLDivElement>(null);

  // The result banner sits at the top of a scrolling form; bring it into view.
  useEffect(() => {
    if (testResult) resultRef.current?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [testResult]);

  const save = useMutation({
    mutationFn: (d: NotificationDraft) => (initial ? notificationsApi.update(initial.id, d) : notificationsApi.create(d)),
    meta: { silent: true },
    onSuccess: (n) => {
      client.setQueryData<Notification[]>(keys.notifications, (l) =>
        l ? (initial ? l.map((x) => (x.id === n.id ? n : x)) : [...l, n]) : l,
      );
      void client.invalidateQueries({ queryKey: keys.notifications });
      toast.success(initial ? 'Connection saved' : 'Connection added', n.name);
      onClose();
    },
  });

  const test = useMutation({
    mutationFn: (d: NotificationDraft) => notificationsApi.test(d),
    meta: { silent: true },
    onMutate: () => setTestResult(null),
    onSuccess: setTestResult,
    onError: (e) => setTestResult({ ok: false, message: errorMessage(e) }),
  });

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    setShowErrors(true);
    if (Object.keys(validate(draft)).length) return;
    save.mutate(clean(draft));
  };

  const onTest = () => {
    setShowErrors(true);
    if (Object.keys(validate(draft)).length) return;
    test.mutate(clean(draft));
  };

  // Typed setters per kind keep the config shape honest.
  const setConfig = <C extends object>(patch: Partial<C>) =>
    setDraft((d) => ({ ...d, config: { ...d.config, ...patch } }) as NotificationDraft);

  return (
    <Modal
      open
      onClose={onClose}
      title={initial ? `Edit ${initial.name}` : `Add ${meta.label}`}
      description={meta.description}
      size="lg"
      footer={
        <>
          <Button variant="ghost" icon={<FlaskConical size={16} />} onClick={onTest} loading={test.isPending} className={styles.testButton}>
            Test
          </Button>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" form="notification-form" loading={save.isPending}>
            {initial ? 'Save' : 'Add connection'}
          </Button>
        </>
      }
    >
      <form id="notification-form" className={styles.form} onSubmit={onSubmit} noValidate>
        {testResult && (
          <div ref={resultRef}>
            <Banner tone={testResult.ok ? 'success' : 'danger'} title={testResult.ok ? 'Test succeeded' : 'Test failed'}>
              {testResult.message}
            </Banner>
          </div>
        )}
        {save.isError && <Banner tone="danger" title="Could not save">{errorMessage(save.error)}</Banner>}

        <FieldRow>
          <FormField label="Name" error={errors.name}>
            {(p) => <TextInput {...p} value={draft.name} onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))} />}
          </FormField>
        </FieldRow>
        <Switch label="Enabled" checked={draft.enabled} onChange={(enabled) => setDraft((d) => ({ ...d, enabled }))} />

        {draft.kind === 'webhook' && <WebhookFields config={draft.config} setConfig={setConfig<WebhookConfig>} errors={errors} />}
        {draft.kind === 'apprise' && <AppriseFields config={draft.config} setConfig={setConfig<AppriseConfig>} errors={errors} />}
        {draft.kind === 'audiobookshelf' && <AbsFields config={draft.config} setConfig={setConfig<AbsConfig>} errors={errors} />}

        {draft.kind !== 'audiobookshelf' && draft.events && (
          <Fieldset legend="Events" description="Which events send a notification.">
            <div className={styles.events}>
              {HISTORY_EVENTS.map((ev) => (
                <Checkbox
                  key={ev}
                  label={HISTORY_EVENT[ev].label}
                  checked={draft.events?.includes(ev) ?? false}
                  onChange={(e) =>
                    setDraft((d) => {
                      const events = d.events ?? [];
                      return {
                        ...d,
                        events: e.target.checked ? [...events, ev] : events.filter((x) => x !== ev),
                      } as NotificationDraft;
                    })
                  }
                />
              ))}
            </div>
            {errors.events && <p className={styles.error}>{errors.events}</p>}
          </Fieldset>
        )}
      </form>
    </Modal>
  );
}

function SecretInput({
  value,
  onChange,
  ...p
}: {
  value: string;
  onChange: (v: string) => void;
  id: string;
  'aria-describedby'?: string;
  'aria-invalid'?: true;
}) {
  const stored = value === SECRET_MASK;
  return (
    <TextInput
      {...p}
      type="password"
      autoComplete="new-password"
      value={value}
      onFocus={(e) => stored && e.target.select()}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

const SECRET_HINT = 'Stored securely; shown masked. Type a new value to replace it.';

function WebhookFields({
  config,
  setConfig,
  errors,
}: {
  config: WebhookConfig;
  setConfig: (p: Partial<WebhookConfig>) => void;
  errors: Errors;
}) {
  const setHeader = (i: number, patch: Partial<{ key: string; value: string }>) =>
    setConfig({ headers: config.headers.map((h, j) => (j === i ? { ...h, ...patch } : h)) });
  return (
    <Fieldset legend="Webhook">
      <div className={styles.urlRow}>
        <FormField label="URL" error={errors.url}>
          {(p) => (
            <TextInput {...p} type="url" placeholder="https://example.com/hooks/librisync" value={config.url} onChange={(e) => setConfig({ url: e.target.value })} />
          )}
        </FormField>
        <FormField label="Method">
          {(p) => (
            <Select {...p} value={config.method} onChange={(e) => setConfig({ method: e.target.value as WebhookConfig['method'] })}>
              <option value="POST">POST</option>
              <option value="PUT">PUT</option>
            </Select>
          )}
        </FormField>
      </div>
      <FieldRow>
        <FormField label="Username" hint="Optional HTTP basic auth.">
          {(p) => <TextInput {...p} autoComplete="off" value={config.username ?? ''} onChange={(e) => setConfig({ username: e.target.value })} />}
        </FormField>
        <FormField label="Password" hint={config.password === SECRET_MASK ? SECRET_HINT : undefined}>
          {(p) => <SecretInput {...p} value={config.password ?? ''} onChange={(password) => setConfig({ password })} />}
        </FormField>
      </FieldRow>
      <div className={styles.headers}>
        <p className={styles.subLabel}>Headers</p>
        {config.headers.length === 0 && <p className={styles.hint}>No extra headers.</p>}
        {config.headers.map((h, i) => (
          <div key={i} className={styles.headerRow}>
            <TextInput aria-label={`Header ${i + 1} name`} placeholder="Name" value={h.key} onChange={(e) => setHeader(i, { key: e.target.value })} />
            <TextInput aria-label={`Header ${i + 1} value`} placeholder="Value" value={h.value} onChange={(e) => setHeader(i, { value: e.target.value })} />
            <Button
              iconOnly
              variant="ghost"
              label={`Remove header ${i + 1}`}
              icon={<Trash2 size={16} />}
              onClick={() => setConfig({ headers: config.headers.filter((_, j) => j !== i) })}
            />
          </div>
        ))}
        {errors.headers && <p className={styles.error}>{errors.headers}</p>}
        <div>
          <Button size="sm" icon={<Plus size={14} />} onClick={() => setConfig({ headers: [...config.headers, { key: '', value: '' }] })}>
            Add header
          </Button>
        </div>
      </div>
    </Fieldset>
  );
}

function AppriseFields({
  config,
  setConfig,
  errors,
}: {
  config: AppriseConfig;
  setConfig: (p: Partial<AppriseConfig>) => void;
  errors: Errors;
}) {
  return (
    <Fieldset
      legend="Apprise"
      description="Needs an apprise-api server. Use a configuration key saved on that server, or list Apprise URLs here directly."
    >
      <FormField label="Server URL" error={errors.serverUrl}>
        {(p) => (
          <TextInput {...p} type="url" placeholder="http://apprise:8000" value={config.serverUrl} onChange={(e) => setConfig({ serverUrl: e.target.value })} />
        )}
      </FormField>
      <FieldRow>
        <FormField label="Configuration key" error={errors.configKey}>
          {(p) => <TextInput {...p} value={config.configKey ?? ''} onChange={(e) => setConfig({ configKey: e.target.value })} />}
        </FormField>
        <FormField label="Tags" hint="Optional; only notify services with these tags.">
          {(p) => <TextInput {...p} value={config.tags ?? ''} onChange={(e) => setConfig({ tags: e.target.value })} />}
        </FormField>
      </FieldRow>
      <FormField label="Stateless URLs" hint="Instead of a configuration key: one or more Apprise URLs, separated by spaces or commas.">
        {(p) => (
          <Textarea
            {...p}
            rows={3}
            spellCheck={false}
            placeholder="discord://webhook_id/webhook_token ntfys://ntfy.sh/topic"
            value={config.statelessUrls ?? ''}
            onChange={(e) => setConfig({ statelessUrls: e.target.value })}
          />
        )}
      </FormField>
    </Fieldset>
  );
}

function AbsFields({
  config,
  setConfig,
  errors,
}: {
  config: AbsConfig;
  setConfig: (p: Partial<AbsConfig>) => void;
  errors: Errors;
}) {
  const [libraries, setLibraries] = useState<AbsLibrary[] | null>(null);
  const fetchLibraries = useMutation({
    mutationFn: () => notificationsApi.absLibraries(config.url.trim(), config.apiKey),
    meta: { silent: true },
    onSuccess: (libs) => {
      setLibraries(libs);
      if (libs.length === 1 && libs[0]) setConfig({ libraryId: libs[0].id });
    },
  });
  const keyIsMasked = config.apiKey === SECRET_MASK;
  const canFetch = isHttpUrl(config.url) && config.apiKey.trim() !== '' && !keyIsMasked;
  const selected = libraries?.find((l) => l.id === config.libraryId);

  return (
    <Fieldset
      legend="Audiobookshelf"
      description="After each downloaded book LibriSync asks Audiobookshelf to scan the library, debounced to once a minute. Audiobookshelf must see the same files: mount the LibriSync library folder into it."
    >
      <FieldRow>
        <FormField label="Audiobookshelf URL" error={errors.url}>
          {(p) => (
            <TextInput {...p} type="url" placeholder="http://audiobookshelf:80" value={config.url} onChange={(e) => setConfig({ url: e.target.value })} />
          )}
        </FormField>
        <FormField
          label="API key"
          error={errors.apiKey}
          hint={keyIsMasked ? SECRET_HINT : 'Audiobookshelf → Settings → Users → your user → API token.'}
        >
          {(p) => <SecretInput {...p} value={config.apiKey} onChange={(apiKey) => setConfig({ apiKey })} />}
        </FormField>
      </FieldRow>

      <div className={styles.libraryRow}>
        <Button icon={<Library size={16} />} onClick={() => fetchLibraries.mutate()} disabled={!canFetch} loading={fetchLibraries.isPending}>
          Fetch libraries
        </Button>
        {keyIsMasked && <p className={styles.hint}>Re-enter the API key to fetch the library list.</p>}
      </div>
      {fetchLibraries.isError && (
        <Banner tone="danger" title="Could not reach Audiobookshelf">
          {errorMessage(fetchLibraries.error)}
        </Banner>
      )}

      {libraries ? (
        libraries.length === 0 ? (
          <Banner tone="warning" title="No libraries found">
            This Audiobookshelf has no libraries the API key can see.
          </Banner>
        ) : (
          <FormField label="Library" error={errors.libraryId}>
            {(p) => (
              <Select {...p} value={config.libraryId} onChange={(e) => setConfig({ libraryId: e.target.value })}>
                <option value="" disabled>
                  Pick a library
                </option>
                {libraries.map((l) => (
                  <option key={l.id} value={l.id}>
                    {l.name}
                  </option>
                ))}
              </Select>
            )}
          </FormField>
        )
      ) : (
        config.libraryId && (
          <p className={styles.hint}>
            Library id <code>{config.libraryId}</code>. Fetch libraries to change it.
          </p>
        )
      )}
      {errors.libraryId && !libraries && <p className={styles.error}>{errors.libraryId}</p>}
      {selected && selected.folders.length > 0 && (
        <p className={styles.hint}>Folders in this library: {selected.folders.join(', ')}</p>
      )}

      <FormField
        label="Path prefix"
        hint="Optional. Where Audiobookshelf sees LibriSync's library folder, if it is mounted at a different path, e.g. /audiobooks."
      >
        {(p) => <TextInput {...p} value={config.pathPrefix ?? ''} onChange={(e) => setConfig({ pathPrefix: e.target.value })} />}
      </FormField>
    </Fieldset>
  );
}
