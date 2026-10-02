/**
 * Warns that one or more Audible accounts were signed in before the v0.0.30
 * device-registration fix, so Audible rate-limits them and their downloads fall
 * back to the legacy AAX path, whose files are far larger and so slower to
 * download. Shown on the Library list, dismissible permanently. The Accounts tab
 * instead marks each affected account with a warning icon that opens the same
 * explanation (showLegacyAccountInfo).
 *
 * Visibility is decided by the parent (via hasLegacyAudibleAccounts); this
 * component only renders the banner.
 */
import React from 'react';
import { View, Text, TouchableOpacity, Alert } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';

interface Props {
  /** Show a dismiss (×) button. The parent persists the dismissal. */
  dismissible?: boolean;
  onDismiss?: () => void;
}

const INFO_TITLE = 'Why some downloads are slower';
const INFO_BODY =
  "One or more of your Audible accounts were signed in before an app update that " +
  "changed how the app registers with Audible.\n\n" +
  "Audible now rate-limits the old registration, so downloads for those accounts " +
  "fall back to a slower legacy method and may occasionally fail.\n\n" +
  "To fix it: open the Accounts tab, sign out of the affected Audible account, then " +
  "sign in again. Your library, settings, and already-downloaded books are kept — " +
  "only the sign-in is redone.";

/** The shared explanation, used by this banner and the per-account warning badge. */
export function showLegacyAccountInfo() {
  Alert.alert(INFO_TITLE, INFO_BODY, [{ text: 'Got it' }]);
}

export default function LegacyAccountBanner({ dismissible = false, onDismiss }: Props) {
  const styles = useStyles(createStyles);
  const { colors } = useTheme();

  return (
    <View style={styles.container}>
      <Ionicons name="warning" size={20} color={colors.warning} style={styles.leadIcon} />
      <Text style={styles.message}>
        Some Audible accounts need to be signed in again for full-speed downloads.
      </Text>
      <TouchableOpacity
        onPress={showLegacyAccountInfo}
        hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
        accessibilityLabel="Why downloads are slower"
        style={styles.iconButton}
      >
        <Ionicons name="information-circle-outline" size={22} color={colors.accent} />
      </TouchableOpacity>
      {dismissible && (
        <TouchableOpacity
          onPress={onDismiss}
          hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
          accessibilityLabel="Dismiss"
          style={styles.iconButton}
        >
          <Ionicons name="close" size={20} color={colors.textSecondary} />
        </TouchableOpacity>
      )}
    </View>
  );
}

const createStyles = (theme: Theme) => ({
  container: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    backgroundColor: theme.colors.backgroundSecondary,
    borderLeftWidth: 4,
    borderLeftColor: theme.colors.warning,
    borderRadius: 8,
    paddingVertical: theme.spacing.sm,
    paddingHorizontal: theme.spacing.md,
    marginHorizontal: theme.spacing.md,
    // Library-only: the book list below adds its own md top padding, so give the
    // banner an equal md gap above and no bottom margin — symmetric breathing room.
    marginTop: theme.spacing.md,
    marginBottom: 0,
    gap: theme.spacing.sm,
  },
  leadIcon: {
    marginRight: theme.spacing.xs,
  },
  message: {
    flex: 1,
    ...theme.typography.caption,
    color: theme.colors.textPrimary,
  },
  iconButton: {
    padding: theme.spacing.xs,
  },
});
