import { useEffect, useMemo, useRef, useState } from 'react';
import { useBlocker } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { isApiError } from '../../api/client';
import { settingsApi } from '../../api/endpoints';
import { keys, useSettings } from '../../api/queries';
import type { Settings } from '../../api/types';
import { Button } from '../../components/Button';
import { useConfirm } from '../../components/Confirm';
import { Banner, ErrorState, SkeletonRows } from '../../components/Feedback';
import { PageHeader } from '../../components/Navigation';
import { toast } from '../../components/Toast';
import {
  AudibleSection,
  BackfillSection,
  DownloadsSection,
  GeneralSection,
  MediaSection,
  SchedulingSection,
  SECTIONS,
  type SetField,
} from './SettingsSections';
import { PasswordCard } from './PasswordCard';
import { validateSettings, type FieldErrors } from './validate';
import styles from './SettingsPage.module.css';

export function SettingsPage() {
  const settings = useSettings();

  return (
    <>
      <PageHeader title="Settings" subtitle="How LibriSync names, paces and schedules your downloads." />
      {settings.isPending ? (
        <SkeletonRows rows={6} label="Loading settings" />
      ) : settings.isError ? (
        <ErrorState error={settings.error} onRetry={() => void settings.refetch()} />
      ) : (
        <SettingsForm saved={settings.data} />
      )}
    </>
  );
}

function SettingsForm({ saved }: { saved: Settings }) {
  const client = useQueryClient();
  const confirm = useConfirm();
  const [draft, setDraft] = useState<Settings>(saved);
  const [showErrors, setShowErrors] = useState(false);
  const [serverError, setServerError] = useState<{ field: string | null; message: string } | null>(null);

  // A save elsewhere (or the API key regenerating) replaces the baseline; keep local edits
  // only when there are any.
  const dirty = useMemo(() => JSON.stringify(draft) !== JSON.stringify(saved), [draft, saved]);
  useEffect(() => {
    if (!dirty) setDraft(saved);
    // Only a new baseline should reset the draft.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saved]);

  // Leaving the page inside the app asks first; closing or reloading the tab gets the
  // browser's own prompt.
  const blocker = useBlocker(({ currentLocation, nextLocation }) => dirty && currentLocation.pathname !== nextLocation.pathname);
  const blockerRef = useRef(blocker);
  blockerRef.current = blocker;
  // Keyed on the state alone: the blocker object is new on every render, and asking twice
  // would cancel the first prompt.
  useEffect(() => {
    if (blocker.state !== 'blocked') return;
    void confirm({
      title: 'Discard unsaved changes?',
      message: <p>Your settings changes have not been saved.</p>,
      confirmLabel: 'Discard changes',
      tone: 'danger',
    }).then((ok) => (ok ? blockerRef.current.proceed?.() : blockerRef.current.reset?.()));
  }, [blocker.state, confirm]);

  useEffect(() => {
    if (!dirty) return;
    const warn = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty]);

  const clientErrors = validateSettings(draft);
  const errors: FieldErrors = showErrors ? { ...clientErrors } : {};
  if (serverError?.field) errors[serverError.field] = serverError.message;
  const errorCount = Object.keys(clientErrors).length;

  const set: SetField = (section, key, value) => {
    setServerError(null);
    setDraft((d) => ({ ...d, [section]: { ...d[section], [key]: value } }));
  };

  const save = useMutation({
    mutationFn: settingsApi.put,
    meta: { silent: true },
    onSuccess: (next) => {
      client.setQueryData(keys.settings, next);
      setDraft(next);
      setShowErrors(false);
      toast.success('Settings saved');
    },
    onError: (e) => {
      if (isApiError(e) && e.error === 'invalid_input') {
        const field = e.field;
        setServerError({ field, message: e.message });
        // The field may be scrolled out of view; say so where the user is looking.
        if (field) toast.error('The server rejected a setting', e.message);
      } else {
        toast.error('Could not save settings', e instanceof Error ? e.message : undefined);
      }
    },
  });

  const onSave = () => {
    setShowErrors(true);
    if (errorCount > 0) {
      toast.error('Fix the highlighted fields first');
      return;
    }
    save.mutate(draft);
  };

  const onDiscard = () => {
    setDraft(saved);
    setShowErrors(false);
    setServerError(null);
  };

  const props = { draft, set, errors };

  return (
    <div className={styles.layout}>
      <nav className={styles.toc} aria-label="Settings sections">
        <ul>
          {SECTIONS.map((s) => (
            <li key={s.id}>
              <a href={`#${s.id}`}>{s.title}</a>
            </li>
          ))}
          <li>
            <a href="#password">Password</a>
          </li>
        </ul>
      </nav>

      <form
        className={styles.form}
        onSubmit={(e) => {
          e.preventDefault();
          onSave();
        }}
        noValidate
      >
        {serverError && !serverError.field && (
          <Banner tone="danger" title="The server rejected these settings">
            {serverError.message}
          </Banner>
        )}
        <MediaSection {...props} />
        <SchedulingSection {...props} />
        <BackfillSection {...props} />
        <AudibleSection {...props} />
        <DownloadsSection {...props} />
        <GeneralSection {...props} />

        {dirty && (
          <div className={styles.saveBar} role="region" aria-label="Unsaved changes">
            <p className={styles.saveText}>
              {showErrors && errorCount > 0
                ? `${errorCount} field${errorCount === 1 ? '' : 's'} need attention`
                : 'Unsaved changes'}
            </p>
            <div className={styles.saveActions}>
              <Button variant="ghost" onClick={onDiscard} disabled={save.isPending}>
                Discard
              </Button>
              <Button type="submit" variant="primary" loading={save.isPending}>
                Save changes
              </Button>
            </div>
          </div>
        )}
      </form>
      <div className={styles.after}>
        <PasswordCard />
      </div>
    </div>
  );
}
