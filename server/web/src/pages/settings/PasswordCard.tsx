import { useState, type FormEvent } from 'react';
import { useMutation } from '@tanstack/react-query';
import { errorMessage, isApiError } from '../../api/client';
import { authApi } from '../../api/endpoints';
import { Button } from '../../components/Button';
import { Card } from '../../components/Card';
import { Banner } from '../../components/Feedback';
import { FormField, TextInput } from '../../components/Form';
import { toast } from '../../components/Toast';
import styles from './SettingsSections.module.css';

const MIN = 8;

/** Separate from the settings form: its own endpoint, its own submit. */
export function PasswordCard() {
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [confirm, setConfirm] = useState('');
  const [touched, setTouched] = useState(false);

  const change = useMutation({
    mutationFn: () => authApi.changePassword({ currentPassword: current, newPassword: next }),
    meta: { silent: true },
    onSuccess: () => {
      setCurrent('');
      setNext('');
      setConfirm('');
      setTouched(false);
      toast.success('Password changed');
    },
  });

  const nextError = touched && next.length < MIN ? `At least ${MIN} characters.` : null;
  const confirmError = touched && confirm !== next ? 'The passwords do not match.' : null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    setTouched(true);
    if (!current || next.length < MIN || confirm !== next) return;
    change.mutate();
  };

  // A wrong current password is `400 invalid_input` with field "currentPassword"; it
  // belongs on that field. Anything else is shown above the form.
  const wrongCurrent =
    isApiError(change.error) && change.error.error === 'invalid_input' && change.error.field === 'currentPassword';
  const failure = change.error && !wrongCurrent ? errorMessage(change.error) : null;
  const currentError = wrongCurrent
    ? 'The current password is wrong.'
    : touched && !current
      ? 'Enter your current password.'
      : null;

  return (
    <Card id="password" title="Password" description="The password for signing in to this web interface.">
      <form className={styles.stack} onSubmit={submit} noValidate>
        {failure && <Banner tone="danger" title={failure} />}
        <FormField label="Current password" error={currentError}>
          {(p) => (
            <TextInput
              {...p}
              type="password"
              autoComplete="current-password"
              value={current}
              onChange={(e) => {
                setCurrent(e.target.value);
                if (wrongCurrent) change.reset();
              }}
            />
          )}
        </FormField>
        <FormField label="New password" error={nextError} hint={`At least ${MIN} characters.`}>
          {(p) => <TextInput {...p} type="password" autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} />}
        </FormField>
        <FormField label="Confirm new password" error={confirmError}>
          {(p) => <TextInput {...p} type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />}
        </FormField>
        <div>
          <Button type="submit" loading={change.isPending}>
            Change password
          </Button>
        </div>
      </form>
    </Card>
  );
}
