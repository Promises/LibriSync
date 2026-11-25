import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ArrowLeft, ExternalLink, FileJson, KeyRound, Mail, Smartphone } from 'lucide-react';
import { keys, useProviders } from '../../api/queries';
import type { AccountView, LoginMethod, ProviderInfo } from '../../api/types';
import { Badge } from '../../components/Badge';
import { ChoiceIcon, ChoiceList } from '../../components/ChoiceList';
import { ErrorState, SkeletonRows } from '../../components/Feedback';
import { Modal } from '../../components/Modal';
import { ProviderIcon } from '../../components/ProviderIcon';
import { toast } from '../../components/Toast';
import { CredentialsFlow, DeviceCodeFlow, ExternalBrowserFlow, ImportFlow } from './LoginFlows';
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
  import: {
    title: 'Import from the LibriSync app',
    description: 'Paste an account exported from the LibriSync mobile app. No new device registration.',
    Icon: FileJson,
  },
};

const METHOD_ORDER: LoginMethod[] = ['deviceCode', 'credentials', 'externalBrowser', 'import'];

type Step = { kind: 'provider' } | { kind: 'method'; provider: ProviderInfo } | { kind: 'flow'; provider: ProviderInfo; method: LoginMethod };

export function AddAccountDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [step, setStep] = useState<Step>({ kind: 'provider' });
  const providers = useProviders();
  const client = useQueryClient();

  const close = () => {
    setStep({ kind: 'provider' });
    onClose();
  };

  const onAdded = (account: AccountView) => {
    client.setQueryData<AccountView[]>(keys.accounts, (list) =>
      list ? [...list.filter((a) => a.id !== account.id), account] : [account],
    );
    void client.invalidateQueries({ queryKey: keys.accounts });
    toast.success('Account added', `${account.name}: the first sync has started.`);
    close();
  };

  const pickProvider = (p: ProviderInfo) => {
    const methods = METHOD_ORDER.filter((m) => p.loginMethods.includes(m));
    if (methods.length === 1 && methods[0]) setStep({ kind: 'flow', provider: p, method: methods[0] });
    else setStep({ kind: 'method', provider: p });
  };

  const back = () => {
    if (step.kind === 'flow') {
      const methods = step.provider.loginMethods;
      setStep(methods.length > 1 ? { kind: 'method', provider: step.provider } : { kind: 'provider' });
    } else setStep({ kind: 'provider' });
  };

  const title =
    step.kind === 'provider'
      ? 'Add account'
      : step.kind === 'method'
        ? `Add ${step.provider.name} account`
        : METHOD_INFO[step.method].title;

  return (
    <Modal open={open} onClose={close} title={title} size="md">
      {step.kind !== 'provider' && (
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
        <DeviceCodeFlow provider={step.provider} onAdded={onAdded} />
      )}
      {step.kind === 'flow' && step.method === 'externalBrowser' && (
        <ExternalBrowserFlow provider={step.provider} onAdded={onAdded} />
      )}
      {step.kind === 'flow' && step.method === 'credentials' && (
        <CredentialsFlow provider={step.provider} onAdded={onAdded} />
      )}
      {step.kind === 'flow' && step.method === 'import' && <ImportFlow onAdded={onAdded} />}

      {step.kind === 'provider' && (
        <p className={styles.footnote}>
          <KeyRound size={14} aria-hidden /> Credentials are stored on this server only, in its config volume.
        </p>
      )}
    </Modal>
  );
}
