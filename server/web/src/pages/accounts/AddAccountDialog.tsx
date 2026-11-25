import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ArrowLeft, ExternalLink, KeyRound, Mail, Smartphone } from 'lucide-react';
import { keys, useProviders } from '../../api/queries';
import type { AccountView, LoginMethod, ProviderId, ProviderInfo } from '../../api/types';
import { Badge } from '../../components/Badge';
import { ChoiceIcon, ChoiceList } from '../../components/ChoiceList';
import { ErrorState, SkeletonRows } from '../../components/Feedback';
import { Modal } from '../../components/Modal';
import { ProviderIcon } from '../../components/ProviderIcon';
import { toast } from '../../components/Toast';
import { CredentialsFlow, DeviceCodeFlow, ExternalBrowserFlow } from './LoginFlows';
import styles from './AddAccountDialog.module.css';

const METHOD_INFO: Record<LoginMethod, { title: string; description: string; Icon: typeof Smartphone }> = {
  deviceCode: {
    title: 'Sign in with a code',
    description: 'Enter a short code on the Amazon website from any device. Handles 2-step verification and CAPTCHAs.',
    Icon: Smartphone,
  },
  externalBrowser: {
    title: 'Sign in through the browser',
    description: 'For older Audible-only accounts (not Amazon accounts) that the code sign-in does not cover.',
    Icon: ExternalLink,
  },
  credentials: {
    title: 'Email and password',
    description: 'Sign in with the email and password you use in the store app.',
    Icon: Mail,
  },
};

const METHOD_ORDER: LoginMethod[] = ['deviceCode', 'credentials', 'externalBrowser'];

type Step = { kind: 'provider' } | { kind: 'method'; provider: ProviderInfo } | { kind: 'flow'; provider: ProviderInfo; method: LoginMethod };

/** Signing in again to an existing account: the server replaces its credentials in place. */
export type ReauthTarget = { provider: ProviderId; locale: string | null; name: string };

function firstStep(p: ProviderInfo): Step {
  const methods = METHOD_ORDER.filter((m) => p.loginMethods.includes(m));
  return methods.length === 1 && methods[0] ? { kind: 'flow', provider: p, method: methods[0] } : { kind: 'method', provider: p };
}

export function AddAccountDialog({
  open,
  onClose,
  reauth = null,
}: {
  open: boolean;
  onClose: () => void;
  /** Skip the store picker and sign in to this account again. */
  reauth?: ReauthTarget | null;
}) {
  const [chosen, setStep] = useState<Step>({ kind: 'provider' });
  const providers = useProviders();
  const client = useQueryClient();
  const reauthProvider = reauth ? providers.data?.find((p) => p.id === reauth.provider) : undefined;
  // Re-authentication starts at the account's store; there is no picker to go back to.
  const start = reauthProvider ? firstStep(reauthProvider) : ({ kind: 'provider' } as Step);
  const step: Step = chosen.kind === 'provider' ? start : chosen;
  const atStart = step.kind === start.kind;

  const close = () => {
    setStep({ kind: 'provider' });
    onClose();
  };

  const onAdded = (account: AccountView) => {
    client.setQueryData<AccountView[]>(keys.accounts, (list) =>
      list ? [...list.filter((a) => a.id !== account.id), account] : [account],
    );
    void client.invalidateQueries({ queryKey: keys.accounts });
    if (reauth) toast.success('Signed in again', account.name);
    else toast.success('Account added', `${account.name}: the first sync has started.`);
    close();
  };

  const pickProvider = (p: ProviderInfo) => setStep(firstStep(p));

  const back = () => {
    if (step.kind === 'flow') {
      const methods = step.provider.loginMethods;
      setStep(methods.length > 1 ? { kind: 'method', provider: step.provider } : { kind: 'provider' });
    } else setStep({ kind: 'provider' });
  };

  const title = reauth
    ? `Sign in again: ${reauth.name}`
    : step.kind === 'provider'
      ? 'Add account'
      : step.kind === 'method'
        ? `Add ${step.provider.name} account`
        : METHOD_INFO[step.method].title;

  return (
    <Modal
      open={open}
      onClose={close}
      title={title}
      description={
        reauth ? 'Sign in with the same account. Its settings, history and downloads are kept.' : undefined
      }
      size="md"
    >
      {!atStart && (
        <button type="button" className={styles.back} onClick={back}>
          <ArrowLeft size={16} aria-hidden /> Back
        </button>
      )}

      {step.kind === 'provider' &&
        (providers.isPending ? (
          <SkeletonRows rows={3} label="Loading stores" />
        ) : providers.isError ? (
          <ErrorState error={providers.error} onRetry={() => void providers.refetch()} />
        ) : (
          <>
            <p className={styles.lead}>Which store is the account with?</p>
            <ChoiceList
              onPick={(id) => {
                const p = providers.data.find((x) => x.id === id);
                if (p) pickProvider(p);
              }}
              choices={providers.data.map((p) => ({
                key: p.id,
                icon: <ProviderIcon provider={p.id} size="lg" decorative />,
                title: p.name,
              }))}
            />
          </>
        ))}

      {step.kind === 'method' && (
        <>
          <p className={styles.lead}>How do you want to sign in?</p>
          <ChoiceList
            onPick={(m) => setStep({ kind: 'flow', provider: step.provider, method: m })}
            choices={METHOD_ORDER.filter((m) => step.provider.loginMethods.includes(m)).map((m, i, all) => {
              const info = METHOD_INFO[m];
              return {
                key: m,
                icon: (
                  <ChoiceIcon>
                    <info.Icon size={20} aria-hidden />
                  </ChoiceIcon>
                ),
                title: (
                  <>
                    {info.title} {i === 0 && all.length > 1 && <Badge tone="accent">Recommended</Badge>}
                  </>
                ),
                description: info.description,
              };
            })}
          />
        </>
      )}

      {step.kind === 'flow' && step.method === 'deviceCode' && (
        <DeviceCodeFlow provider={step.provider} onAdded={onAdded} defaultLocale={reauth?.locale} />
      )}
      {step.kind === 'flow' && step.method === 'externalBrowser' && (
        <ExternalBrowserFlow provider={step.provider} onAdded={onAdded} defaultLocale={reauth?.locale} />
      )}
      {step.kind === 'flow' && step.method === 'credentials' && (
        <CredentialsFlow
          provider={step.provider}
          onAdded={onAdded}
          defaultEmail={reauth?.name.includes('@') ? reauth.name : undefined}
        />
      )}

      {step.kind === 'provider' && (
        <p className={styles.footnote}>
          <KeyRound size={14} aria-hidden /> Credentials are stored on this server only, in its config volume.
        </p>
      )}
    </Modal>
  );
}
