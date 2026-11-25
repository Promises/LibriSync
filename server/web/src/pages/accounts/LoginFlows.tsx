import { useEffect, useRef, useState, type FormEvent } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ExternalLink, RotateCw } from 'lucide-react';
import { errorMessage } from '../../api/client';
import { loginApi } from '../../api/endpoints';
import type { AccountView, DeviceCodeSession, ProviderInfo } from '../../api/types';
import { Button, LinkButton } from '../../components/Button';
import { CopyButton } from '../../components/CopyButton';
import { Banner } from '../../components/Feedback';
import { FormField, Select, TextInput, Textarea } from '../../components/Form';
import { Spinner } from '../../components/Spinner';
import { useNow } from '../../hooks/useNow';
import { formatDuration } from '../../lib/format';
import styles from './LoginFlows.module.css';

type FlowProps = {
  provider: ProviderInfo;
  onAdded: (account: AccountView) => void;
  /** Re-authentication: the account's own marketplace. */
  defaultLocale?: string | null;
};

const POLL_MS = 4_000;

/** The given locale if the store has it, else the one matching the browser's region, else the first. */
function guessLocale(provider: ProviderInfo, preferred?: string | null): string {
  if (preferred && provider.locales.some((l) => l.code === preferred)) return preferred;
  const region = navigator.language.split('-')[1]?.toLowerCase();
  const aliases: Record<string, string> = { gb: 'uk' };
  const wanted = region ? (aliases[region] ?? region) : undefined;
  return provider.locales.find((l) => l.code.toLowerCase() === wanted)?.code ?? provider.locales[0]?.code ?? '';
}

function LocalePicker({ provider, value, onChange }: { provider: ProviderInfo; value: string; onChange: (v: string) => void }) {
  return (
    <FormField label="Marketplace" hint="The Audible store your account belongs to, e.g. audible.co.uk for the United Kingdom.">
      {(p) => (
        <Select {...p} value={value} onChange={(e) => onChange(e.target.value)}>
          {provider.locales.map((l) => (
            <option key={l.code} value={l.code}>
              {l.name}
            </option>
          ))}
        </Select>
      )}
    </FormField>
  );
}

function hostAndPath(url: string): string {
  try {
    const u = new URL(url);
    return `${u.host.replace(/^www\./, '')}${u.pathname.replace(/\/$/, '')}`;
  } catch {
    return url;
  }
}

// ---- Audible: device code -----------------------------------------------------------

export function DeviceCodeFlow({ provider, onAdded, defaultLocale }: FlowProps) {
  const [locale, setLocale] = useState(() => guessLocale(provider, defaultLocale));
  const [sessionId, setSessionId] = useState<string | null>(null);
  const sessionRef = useRef<string | null>(null);
  const finished = useRef(false);
  const now = useNow();

  const start = useMutation({
    mutationFn: () => loginApi.startDeviceCode(locale),
    meta: { silent: true },
    onSuccess: (s) => {
      sessionRef.current = s.sessionId;
      setSessionId(s.sessionId);
    },
  });

  const poll = useQuery({
    queryKey: ['deviceCode', sessionId],
    queryFn: () => loginApi.pollDeviceCode(sessionId as string),
    enabled: sessionId !== null,
    initialData: start.data,
    refetchInterval: (q) => (q.state.data?.status === 'pending' ? POLL_MS : false),
    refetchIntervalInBackground: true,
    retry: 3,
    gcTime: 0,
  });

  const session: DeviceCodeSession | undefined = poll.data ?? start.data;

  useEffect(() => {
    if (session?.status === 'registered' && session.account && !finished.current) {
      finished.current = true;
      sessionRef.current = null;
      onAdded(session.account);
    }
  }, [session, onAdded]);

  // Leaving the dialog with a code still pending cancels the session on the server.
  useEffect(
    () => () => {
      if (sessionRef.current) void loginApi.cancelDeviceCode(sessionRef.current).catch(() => undefined);
    },
    [],
  );

  const restart = () => {
    if (sessionRef.current) void loginApi.cancelDeviceCode(sessionRef.current).catch(() => undefined);
    sessionRef.current = null;
    setSessionId(null);
    start.reset();
    start.mutate();
  };

  if (!session) {
    return (
      <div className={styles.flow}>
        <p className={styles.lead}>
          You will get a short code to enter on Amazon's website, on this computer or your phone. Signing in happens on
          Amazon's own page, so 2-step verification works as usual.
        </p>
        {provider.locales.length > 0 && <LocalePicker provider={provider} value={locale} onChange={setLocale} />}
        {start.isError && <Banner tone="danger" title="Could not get a code">{errorMessage(start.error)}</Banner>}
        <div className={styles.actions}>
          <Button variant="primary" onClick={() => start.mutate()} loading={start.isPending}>
            Get sign-in code
          </Button>
        </div>
      </div>
    );
  }

  const remaining = (new Date(session.expiresAt).getTime() - now) / 1000;
  const expired = session.status === 'expired' || (session.status === 'pending' && remaining <= 0);
  const site = hostAndPath(session.verificationUrl);

  if (session.status === 'error') {
    return (
      <div className={styles.flow}>
        <Banner tone="danger" title="Sign-in failed">
          {session.message ?? 'Amazon did not complete the registration.'}
        </Banner>
        <div className={styles.actions}>
          <Button variant="primary" icon={<RotateCw size={16} />} onClick={restart} loading={start.isPending}>
            Try again
          </Button>
        </div>
      </div>
    );
  }

  if (expired) {
    return (
      <div className={styles.flow}>
        <Banner tone="warning" title="The code expired">
          Codes are valid for a few minutes. Get a new one and enter it on {site}.
        </Banner>
        <div className={styles.actions}>
          <Button variant="primary" icon={<RotateCw size={16} />} onClick={restart} loading={start.isPending}>
            Get a new code
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.flow}>
      <ol className={styles.steps}>
        <li>
          Open{' '}
          <a href={session.verificationUrl} target="_blank" rel="noopener noreferrer">
            {site}
          </a>{' '}
          in any browser, on this computer or your phone.
        </li>
        <li>Sign in with the Amazon account you use for Audible.</li>
        <li>Enter this code:</li>
      </ol>

      <div className={styles.codeBox}>
        <output className={styles.code} aria-label="Sign-in code">
          {session.publicCode}
        </output>
        <CopyButton text={session.publicCode} label="Copy code" />
      </div>

      <div className={styles.actions}>
        <LinkButton
          variant="primary"
          href={session.verificationUrl}
          target="_blank"
          rel="noopener noreferrer"
          icon={<ExternalLink size={16} />}
        >
          Open {site}
        </LinkButton>
      </div>

      <p className={styles.waiting} role="status">
        <Spinner size={14} />
        Waiting for you to enter the code… expires in {formatDuration(remaining)}.
      </p>
      {poll.isError && (
        <p className={styles.hint}>Lost contact with the server ({errorMessage(poll.error)}); still trying.</p>
      )}
    </div>
  );
}

// ---- Audible: external browser ------------------------------------------------------

export function ExternalBrowserFlow({ provider, onAdded, defaultLocale }: FlowProps) {
  const [locale, setLocale] = useState(() => guessLocale(provider, defaultLocale));
  const [responseUrl, setResponseUrl] = useState('');

  const start = useMutation({
    mutationFn: () => loginApi.startExternal(locale),
    meta: { silent: true },
  });
  const finish = useMutation({
    mutationFn: () => loginApi.finishExternal(start.data?.sessionId as string, responseUrl.trim()),
    meta: { silent: true },
    onSuccess: onAdded,
  });

  if (!start.data) {
    return (
      <div className={styles.flow}>
        <p className={styles.lead}>
          Use this only if the code sign-in does not accept your account. It is meant for older Audible accounts that
          were never merged into an Amazon account.
        </p>
        {provider.locales.length > 0 && <LocalePicker provider={provider} value={locale} onChange={setLocale} />}
        {start.isError && <Banner tone="danger" title="Could not start sign-in">{errorMessage(start.error)}</Banner>}
        <div className={styles.actions}>
          <Button variant="primary" onClick={() => start.mutate()} loading={start.isPending}>
            Continue
          </Button>
        </div>
      </div>
    );
  }

  const looksRight = /maplanding|openid\.oa2\.authorization_code/.test(responseUrl);
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (responseUrl.trim()) finish.mutate();
  };

  return (
    <form className={styles.flow} onSubmit={onSubmit}>
      <ol className={styles.steps}>
        <li>
          Open the sign-in page and sign in.
          <div className={styles.inlineAction}>
            <LinkButton href={start.data.url} target="_blank" rel="noopener noreferrer" icon={<ExternalLink size={16} />}>
              Open sign-in page
            </LinkButton>
          </div>
        </li>
        <li>
          After signing in you land on a page that says <strong>“Page not found”</strong> or similar. That is expected —
          the page itself does not matter, only its address.
        </li>
        <li>Copy the whole address from the browser's address bar and paste it below.</li>
      </ol>
      <FormField
        label="Address of the final page"
        hint="It starts with https://www.amazon… and contains “maplanding”. The session is valid for 15 minutes."
        error={responseUrl.trim() && !looksRight ? 'This does not look like the final sign-in address.' : null}
      >
        {(p) => (
          <Textarea
            {...p}
            rows={4}
            spellCheck={false}
            placeholder="https://www.amazon.co.uk/ap/maplanding?openid.oa2.authorization_code=…"
            value={responseUrl}
            onChange={(e) => setResponseUrl(e.target.value)}
          />
        )}
      </FormField>
      {finish.isError && <Banner tone="danger" title="Sign-in failed">{errorMessage(finish.error)}</Banner>}
      <div className={styles.actions}>
        <Button type="submit" variant="primary" loading={finish.isPending} disabled={!responseUrl.trim()}>
          Add account
        </Button>
      </div>
    </form>
  );
}

// ---- Libro.fm / Chirp: email + password ---------------------------------------------

export function CredentialsFlow({ provider, onAdded, defaultEmail }: FlowProps & { defaultEmail?: string }) {
  const [email, setEmail] = useState(defaultEmail ?? '');
  const [password, setPassword] = useState('');
  const submit = useMutation({
    mutationFn: () =>
      loginApi.credentials({ provider: provider.id as 'librofm' | 'chirp', email: email.trim(), password }),
    meta: { silent: true },
    onSuccess: onAdded,
  });
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (email.trim() && password) submit.mutate();
  };
  return (
    <form className={styles.flow} onSubmit={onSubmit}>
      <p className={styles.lead}>Sign in with your {provider.name} email and password.</p>
      {provider.id === 'chirp' && (
        <p className={styles.hint}>Accounts created with Google or Apple sign-in have no password and cannot be added.</p>
      )}
      <FormField label="Email">
        {(p) => (
          <TextInput {...p} type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} />
        )}
      </FormField>
      <FormField label="Password">
        {(p) => (
          <TextInput
            {...p}
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        )}
      </FormField>
      {submit.isError && <Banner tone="danger" title="Sign-in failed">{errorMessage(submit.error)}</Banner>}
      <div className={styles.actions}>
        <Button type="submit" variant="primary" loading={submit.isPending} disabled={!email.trim() || !password}>
          Add account
        </Button>
      </div>
    </form>
  );
}

// ---- Import from the app ------------------------------------------------------------

export function ImportFlow({ onAdded }: { onAdded: (account: AccountView) => void }) {
  const [json, setJson] = useState('');
  const submit = useMutation({
    mutationFn: () => loginApi.importAccount(json.trim()),
    meta: { silent: true },
    onSuccess: onAdded,
  });
  let parseError: string | null = null;
  if (json.trim()) {
    try {
      const v: unknown = JSON.parse(json);
      if (!v || typeof v !== 'object' || Array.isArray(v)) parseError = 'Expected a single account object.';
    } catch {
      parseError = 'This is not valid JSON.';
    }
  }
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (json.trim() && !parseError) submit.mutate();
  };
  return (
    <form className={styles.flow} onSubmit={onSubmit}>
      <p className={styles.lead}>
        Export the account from the LibriSync app and paste it here. The server reuses the app's device registration,
        so nothing new appears on your Amazon account.
      </p>
      <Banner tone="warning" title="This text is a password">
        The exported account contains the tokens that sign in as you. Paste it here and nowhere else.
      </Banner>
      <FormField label="Account JSON" error={parseError}>
        {(p) => (
          <Textarea
            {...p}
            rows={8}
            spellCheck={false}
            className={styles.mono}
            value={json}
            onChange={(e) => setJson(e.target.value)}
          />
        )}
      </FormField>
      {submit.isError && <Banner tone="danger" title="Import failed">{errorMessage(submit.error)}</Banner>}
      <div className={styles.actions}>
        <Button type="submit" variant="primary" loading={submit.isPending} disabled={!json.trim() || parseError !== null}>
          Import account
        </Button>
      </div>
    </form>
  );
}
