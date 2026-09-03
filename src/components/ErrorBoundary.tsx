/**
 * App-level crash screen.
 *
 * A render error in React Native otherwise leaves a blank screen (release) or a red box
 * (debug), and neither gives the user anything to send. This catches it, shows what
 * broke, and puts a full report — error, stacks, app version, and the last sync report —
 * one tap away on the clipboard.
 */
import React from 'react';
import { Text, ScrollView, Alert, TouchableOpacity, StyleSheet } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { copyTextToClipboard } from '../../modules/expo-rust-bridge';
import { formatCrashReport } from '../services/diagnostics';
import { darkColors, spacing } from '../styles/theme';
import type { ColorScheme } from '../styles/theme';

interface Props {
  children: React.ReactNode;
}

interface State {
  error: Error | null;
  componentStack?: string;
}

export default class ErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('[ErrorBoundary] Caught render error:', error, info.componentStack);
    this.setState({ error, componentStack: info.componentStack ?? undefined });
  }

  private handleCopy = () => {
    const { error, componentStack } = this.state;
    if (!error) return;
    try {
      copyTextToClipboard(formatCrashReport(error, componentStack));
      Alert.alert('Copied', 'Crash details copied — paste them into a bug report.');
    } catch (copyError: any) {
      Alert.alert('Copy Failed', copyError?.message || String(copyError));
    }
  };

  private handleDismiss = () => {
    // Clearing the error re-renders the tree; a transient failure recovers, a
    // deterministic one lands right back here with the same details.
    this.setState({ error: null, componentStack: undefined });
  };

  render() {
    const { error, componentStack } = this.state;
    if (!error) return this.props.children;

    // Fixed dark palette rather than the theme hook: this screen must not depend on
    // context or hooks that could themselves be what just crashed.
    const styles = createStyles(darkColors);

    return (
      <SafeAreaView style={styles.container}>
        <Text style={styles.title}>Something went wrong</Text>
        <Text style={styles.subtitle}>
          LibriSync hit an error it could not recover from. Copying the details and sending
          them along is the fastest way to get it fixed.
        </Text>

        <ScrollView style={styles.detail} contentContainerStyle={styles.detailContent}>
          <Text style={styles.errorText}>
            {error.name}: {error.message}
          </Text>
          {!!error.stack && <Text style={styles.stackText}>{error.stack}</Text>}
          {!!componentStack && <Text style={styles.stackText}>{componentStack}</Text>}
        </ScrollView>

        <TouchableOpacity style={styles.primaryButton} onPress={this.handleCopy}>
          <Text style={styles.primaryButtonText}>Copy Crash Details</Text>
        </TouchableOpacity>
        <TouchableOpacity style={styles.secondaryButton} onPress={this.handleDismiss}>
          <Text style={styles.secondaryButtonText}>Try Again</Text>
        </TouchableOpacity>
      </SafeAreaView>
    );
  }
}

const createStyles = (palette: ColorScheme) =>
  StyleSheet.create({
    container: {
      flex: 1,
      backgroundColor: palette.background,
      padding: spacing.lg,
    },
    title: {
      fontSize: 24,
      fontWeight: '700',
      color: palette.textPrimary,
      marginBottom: 8,
    },
    subtitle: {
      fontSize: 14,
      color: palette.textSecondary,
      marginBottom: 16,
    },
    detail: {
      flex: 1,
      backgroundColor: palette.backgroundSecondary,
      borderRadius: 8,
      marginBottom: 16,
    },
    detailContent: {
      padding: 16,
    },
    errorText: {
      fontSize: 14,
      color: palette.error,
      marginBottom: 12,
    },
    stackText: {
      fontFamily: 'monospace',
      fontSize: 11,
      color: palette.textSecondary,
      marginBottom: 12,
    },
    primaryButton: {
      backgroundColor: palette.accent,
      borderRadius: 8,
      paddingVertical: 14,
      alignItems: 'center',
      marginBottom: 8,
    },
    primaryButtonText: {
      color: palette.background,
      fontSize: 16,
      fontWeight: '600',
    },
    secondaryButton: {
      borderColor: palette.border,
      borderWidth: 1,
      borderRadius: 8,
      paddingVertical: 14,
      alignItems: 'center',
    },
    secondaryButtonText: {
      color: palette.textPrimary,
      fontSize: 16,
    },
  });
