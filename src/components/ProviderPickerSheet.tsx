import React from 'react';
import { View, Text, Modal, TouchableOpacity, ScrollView } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import type { ProviderEntry } from '../services/providers/registry';

interface Props {
  visible: boolean;
  providers: ProviderEntry[];
  onSelect: (provider: ProviderEntry) => void;
  onClose: () => void;
}

/**
 * "Which provider?" bottom sheet shown by Add Account.
 *
 * Renders whatever it is given, so enabling or adding a provider is a registry
 * change rather than a UI change.
 */
export default function ProviderPickerSheet({ visible, providers, onSelect, onClose }: Props) {
  const styles = useStyles(createStyles);
  const { colors, spacing } = useTheme();
  const insets = useSafeAreaInsets();

  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <TouchableOpacity style={styles.overlay} activeOpacity={1} onPress={onClose}>
        <View
          style={[styles.sheet, { paddingBottom: insets.bottom + spacing.lg }]}
          onStartShouldSetResponder={() => true}
        >
          <View style={styles.sheetHeader}>
            <Text style={styles.sheetTitle}>Add Account</Text>
            <TouchableOpacity onPress={onClose} accessibilityLabel="Close">
              <Ionicons name="close" size={24} color={colors.textPrimary} />
            </TouchableOpacity>
          </View>
          <Text style={styles.sheetSubtitle}>Choose an audiobook source to connect.</Text>

          <ScrollView>
            {providers.map((provider) => (
              <TouchableOpacity
                key={provider.id}
                style={styles.providerCard}
                onPress={() => onSelect(provider)}
              >
                <View style={styles.providerIcon}>
                  <Ionicons name={provider.icon} size={28} color={colors[provider.tint]} />
                </View>
                <View style={styles.providerInfo}>
                  <Text style={styles.providerName}>{provider.name}</Text>
                  <Text style={styles.providerDescription}>{provider.description}</Text>
                </View>
                <Ionicons name="chevron-forward" size={20} color={colors.textSecondary} />
              </TouchableOpacity>
            ))}
          </ScrollView>
        </View>
      </TouchableOpacity>
    </Modal>
  );
}

const createStyles = (theme: Theme) => ({
  overlay: {
    flex: 1,
    backgroundColor: 'rgba(0, 0, 0, 0.5)',
    justifyContent: 'flex-end' as const,
  },
  sheet: {
    backgroundColor: theme.colors.background,
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    padding: theme.spacing.lg,
    maxHeight: '75%' as const,
  },
  sheetHeader: {
    flexDirection: 'row' as const,
    justifyContent: 'space-between' as const,
    alignItems: 'center' as const,
  },
  sheetTitle: {
    ...theme.typography.title,
    fontSize: 20,
    flex: 1,
    marginRight: theme.spacing.md,
  },
  sheetSubtitle: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
    marginBottom: theme.spacing.md,
  },
  providerCard: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    backgroundColor: theme.colors.backgroundSecondary,
    borderRadius: 12,
    padding: theme.spacing.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    gap: theme.spacing.md,
    marginBottom: theme.spacing.md,
  },
  providerIcon: {
    width: 48,
    height: 48,
    borderRadius: 12,
    backgroundColor: theme.colors.backgroundTertiary,
    justifyContent: 'center' as const,
    alignItems: 'center' as const,
  },
  providerInfo: {
    flex: 1,
    gap: theme.spacing.xs,
  },
  providerName: {
    ...theme.typography.subtitle,
    fontSize: 18,
  },
  providerDescription: {
    ...theme.typography.caption,
  },
});
