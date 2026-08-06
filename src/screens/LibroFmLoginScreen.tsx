import React, { useState } from 'react';
import { Text, TextInput, TouchableOpacity, ActivityIndicator, Alert, ScrollView } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import Button from '../components/Button';
import { providerLogin, saveAccount } from '../../modules/expo-rust-bridge';
import type { Account } from '../../modules/expo-rust-bridge';
import { getDatabasePath } from '../utils/appPaths';

interface Props {
  /** Called with the saved account once sign-in succeeds. */
  onLoginSuccess: (account: Account) => void;
  /** Provided when adding a further account, so the form can be dismissed. */
  onCancel?: () => void;
  title?: string;
}

/**
 * Libro.fm credential sign-in — this provider's equivalent of `LoginScreen`.
 *
 * Saves the account and hands it back; syncing is the account screen's job, the
 * same split Audible uses. Auth is a plain password grant (see providers/librofm.rs).
 */
export default function LibroFmLoginScreen({ onLoginSuccess, onCancel, title }: Props) {
  const styles = useStyles(createStyles);
  const { colors } = useTheme();

  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);

  const signIn = async () => {
    if (!email.trim() || !password) {
      Alert.alert('Missing info', 'Enter your Libro.fm email and password.');
      return;
    }
    setBusy(true);
    try {
      const dbPath = getDatabasePath();
      const identifier = email.trim();

      // Password grant → credential blob { access_token, username }
      const creds = await providerLogin('librofm', { username: identifier, password });

      const account: Account = {
        account_id: identifier,
        account_name: identifier,
        provider: 'librofm',
        identity: creds as unknown as Account['identity'],
      };
      await saveAccount(dbPath, account);

      // Don't hold the password in component state any longer than needed.
      setPassword('');
      onLoginSuccess(account);
    } catch (e: any) {
      Alert.alert('Libro.fm sign-in failed', e?.message || 'Unknown error');
    } finally {
      setBusy(false);
    }
  };

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.title}>{title || 'Libro.fm'}</Text>
        <Text style={styles.subtitle}>
          Sign in to sync and download your Libro.fm audiobooks. DRM-free.
        </Text>

        <Text style={styles.label}>Email</Text>
        <TextInput
          style={styles.input}
          value={email}
          onChangeText={setEmail}
          placeholder="you@example.com"
          placeholderTextColor={colors.textSecondary}
          autoCapitalize="none"
          keyboardType="email-address"
          autoCorrect={false}
          editable={!busy}
        />

        <Text style={styles.label}>Password</Text>
        <TextInput
          style={styles.input}
          value={password}
          onChangeText={setPassword}
          placeholder="Password"
          placeholderTextColor={colors.textSecondary}
          secureTextEntry
          autoCapitalize="none"
          autoCorrect={false}
          editable={!busy}
        />

        <TouchableOpacity
          style={[styles.button, busy && styles.buttonDisabled]}
          onPress={signIn}
          disabled={busy}
        >
          {busy ? (
            <ActivityIndicator color={colors.background} />
          ) : (
            <Text style={styles.buttonText}>Sign in</Text>
          )}
        </TouchableOpacity>

        {!!onCancel && (
          <Button title="Cancel" onPress={onCancel} variant="outlined" state="primary" disabled={busy} />
        )}

        <Text style={styles.note}>
          Note: accounts that use Google/social sign-in (no password) aren't supported.
        </Text>
      </ScrollView>
    </SafeAreaView>
  );
}

const createStyles = (theme: Theme) => ({
  container: { flex: 1, backgroundColor: theme.colors.background },
  content: { padding: theme.spacing.lg, gap: theme.spacing.md },
  title: { ...theme.typography.title },
  subtitle: { ...theme.typography.caption, marginBottom: theme.spacing.md },
  label: { ...theme.typography.body, fontWeight: '600' as const, marginTop: theme.spacing.sm },
  input: {
    backgroundColor: theme.colors.backgroundSecondary,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: 8,
    paddingHorizontal: theme.spacing.md,
    paddingVertical: theme.spacing.sm,
    color: theme.colors.textPrimary,
    ...theme.typography.body,
  },
  button: {
    backgroundColor: theme.colors.accent,
    borderRadius: 8,
    paddingVertical: theme.spacing.md,
    alignItems: 'center' as const,
    marginTop: theme.spacing.md,
  },
  buttonDisabled: { opacity: 0.5 },
  buttonText: {
    ...theme.typography.body,
    color: theme.colors.background,
    fontWeight: '700' as const,
  },
  note: { ...theme.typography.caption, color: theme.colors.textSecondary, marginTop: theme.spacing.lg },
});
