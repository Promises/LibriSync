import React, { useState } from 'react';
import { View, Text, TextInput, TouchableOpacity, ActivityIndicator, Alert, ScrollView } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import {
  providerLogin,
  providerSyncLibraryPage,
  saveAccount,
} from '../../modules/expo-rust-bridge';
import type { Account } from '../../modules/expo-rust-bridge';
import { getDatabasePath } from '../utils/appPaths';

/**
 * Libro.fm sign-in: email/password → bearer token → save account → sync library.
 * DRM-free store; auth is a plain password grant (see native providers/librofm.rs).
 */
export default function LibroFmLoginScreen() {
  const styles = useStyles(createStyles);
  const { colors } = useTheme();

  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);

  const signInAndSync = async () => {
    if (!email.trim() || !password) {
      Alert.alert('Missing info', 'Enter your Libro.fm email and password.');
      return;
    }
    setBusy(true);
    setStatus('Signing in…');
    try {
      const dbPath = getDatabasePath();

      // 1) Password grant → credential blob { access_token, username }
      const creds = await providerLogin('librofm', { username: email.trim(), password });

      // 2) Persist the account (provider-tagged, opaque credential in identity)
      const account = {
        account_id: email.trim(),
        account_name: email.trim(),
        provider: 'librofm',
        identity: creds,
      } as unknown as Account;
      await saveAccount(dbPath, account);

      // 3) Sync the owned library page by page
      setStatus('Syncing your library…');
      let page = 1;
      let added = 0;
      let updated = 0;
      // Guard against a runaway loop.
      for (let i = 0; i < 500; i++) {
        const stats = await providerSyncLibraryPage('librofm', dbPath, account, page);
        added += stats.books_added;
        updated += stats.books_updated;
        setStatus(`Syncing… ${added + updated} books so far`);
        if (!stats.has_more) break;
        page += 1;
      }

      setStatus(null);
      Alert.alert('Libro.fm connected', `Synced your library.\nAdded ${added}, updated ${updated}.`);
    } catch (e: any) {
      setStatus(null);
      Alert.alert('Libro.fm sign-in failed', e?.message || 'Unknown error');
    } finally {
      setBusy(false);
    }
  };

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.title}>Libro.fm</Text>
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
          onPress={signInAndSync}
          disabled={busy}
        >
          {busy ? (
            <ActivityIndicator color={colors.background} />
          ) : (
            <Text style={styles.buttonText}>Sign in & sync</Text>
          )}
        </TouchableOpacity>

        {status && <Text style={styles.status}>{status}</Text>}

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
  status: { ...theme.typography.caption, textAlign: 'center' as const, marginTop: theme.spacing.sm },
  note: { ...theme.typography.caption, color: theme.colors.textSecondary, marginTop: theme.spacing.lg },
});
