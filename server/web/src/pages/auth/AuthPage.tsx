import { useState, type FormEvent } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { errorMessage, isApiError } from '../../api/client';
import { authApi } from '../../api/endpoints';
import { keys } from '../../api/queries';
import { Button } from '../../components/Button';
import { Banner } from '../../components/Feedback';
import { FormField, TextInput } from '../../components/Form';
import styles from './AuthPage.module.css';

const MIN_PASSWORD = 8;

/** First-run admin creation (`mode="setup"`) or sign-in. */
export function AuthPage({ mode }: { mode: 'setup' | 'login' }) {
  const client = useQueryClient();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [touched, setTouched] = useState(false);

  const submit = useMutation({
    mutationFn: () =>
      mode === 'setup' ? authApi.setup({ username, password }) : authApi.login({ username, password }),
    meta: { silent: true },
    onSuccess: () => client.invalidateQueries({ queryKey: keys.authState }),
  });

  const isSetup = mode === 'setup';
  const passwordError =
    isSetup && touched && password.length < MIN_PASSWORD ? `At least ${MIN_PASSWORD} characters.` : null;
  const confirmError = isSetup && touched && confirm !== password ? 'The passwords do not match.' : null;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    setTouched(true);
    if (!username.trim() || !password) return;
    if (isSetup && (password.length < MIN_PASSWORD || confirm !== password)) return;
    submit.mutate();
  };

  const failure = !submit.error
    ? null
    : mode === 'login' && isApiError(submit.error) && submit.error.status === 401
      ? 'Wrong username or password.'
      : errorMessage(submit.error);

  return (
    <main className={styles.page}>
      <div className={styles.panel}>
        <div className={styles.brand}>
          <img src="/favicon.svg" alt="" width={56} height={56} />
          <h1 className={styles.title}>{isSetup ? 'Welcome to LibriSync Server' : 'Sign in to LibriSync'}</h1>
          <p className={styles.subtitle}>
            {isSetup
              ? 'Create the administrator account. You will use it to sign in to this web interface.'
              : 'Your self-hosted audiobook library.'}
          </p>
        </div>

        <form className={styles.form} onSubmit={onSubmit} noValidate>
          {failure && <Banner tone="danger" title={failure} />}
          <FormField label="Username" error={touched && !username.trim() ? 'Enter a username.' : null}>
            {(p) => (
              <TextInput
                {...p}
                autoComplete="username"
                autoFocus
                value={username}
                onChange={(e) => setUsername(e.target.value)}
              />
            )}
          </FormField>
          <FormField
            label="Password"
            hint={isSetup ? `At least ${MIN_PASSWORD} characters.` : undefined}
            error={passwordError ?? (touched && !password ? 'Enter a password.' : null)}
          >
            {(p) => (
              <TextInput
                {...p}
                type="password"
                autoComplete={isSetup ? 'new-password' : 'current-password'}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            )}
          </FormField>
          {isSetup && (
            <FormField label="Confirm password" error={confirmError}>
              {(p) => (
                <TextInput
                  {...p}
                  type="password"
                  autoComplete="new-password"
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                />
              )}
            </FormField>
          )}
          <Button type="submit" variant="primary" size="lg" loading={submit.isPending}>
            {isSetup ? 'Create account' : 'Sign in'}
          </Button>
        </form>
      </div>
    </main>
  );
}
