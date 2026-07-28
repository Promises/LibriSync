import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';

type ProvidersStackParamList = {
  ProvidersList: undefined;
  Audible: undefined;
  LibriVox: undefined;
  LibroFm: undefined;
};

type Props = {
  navigation: NativeStackNavigationProp<ProvidersStackParamList, 'ProvidersList'>;
};

export default function ProvidersScreen({ navigation }: Props) {
  const styles = useStyles(createStyles);
  const { colors } = useTheme();

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      <View style={styles.header}>
        <Text style={styles.headerTitle}>Providers</Text>
        <Text style={styles.headerSubtitle}>Connect audiobook sources</Text>
      </View>

      <View style={styles.list}>
        <TouchableOpacity
          style={styles.providerCard}
          onPress={() => navigation.navigate('LibriVox')}
        >
          <View style={styles.providerIcon}>
            <Ionicons name="book-outline" size={28} color={colors.success} />
          </View>
          <View style={styles.providerInfo}>
            <Text style={styles.providerName}>LibriVox</Text>
            <Text style={styles.providerDescription}>
              Free public domain audiobooks. No account needed.
            </Text>
          </View>
          <Ionicons name="chevron-forward" size={20} color={colors.textSecondary} />
        </TouchableOpacity>

        <TouchableOpacity
          style={styles.providerCard}
          onPress={() => navigation.navigate('Audible')}
        >
          <View style={styles.providerIcon}>
            <Ionicons name="headset-outline" size={28} color={colors.accent} />
          </View>
          <View style={styles.providerInfo}>
            <Text style={styles.providerName}>Audible</Text>
            <Text style={styles.providerDescription}>
              Sign in to sync and download your Audible library.
            </Text>
          </View>
          <Ionicons name="chevron-forward" size={20} color={colors.textSecondary} />
        </TouchableOpacity>

        <TouchableOpacity
          style={styles.providerCard}
          onPress={() => navigation.navigate('LibroFm')}
        >
          <View style={styles.providerIcon}>
            <Ionicons name="storefront-outline" size={28} color={colors.accent} />
          </View>
          <View style={styles.providerInfo}>
            <Text style={styles.providerName}>Libro.fm</Text>
            <Text style={styles.providerDescription}>
              Sign in to sync and download your Libro.fm library. DRM-free.
            </Text>
          </View>
          <Ionicons name="chevron-forward" size={20} color={colors.textSecondary} />
        </TouchableOpacity>
      </View>
    </SafeAreaView>
  );
}

const createStyles = (theme: Theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  header: {
    padding: theme.spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  headerTitle: {
    ...theme.typography.title,
  },
  headerSubtitle: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
  },
  list: {
    padding: theme.spacing.lg,
    gap: theme.spacing.md,
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
