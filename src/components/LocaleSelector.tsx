import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet, ScrollView } from 'react-native';
import { colors, spacing, typography } from '../styles/theme';
import type { Locale } from '../types/auth';

/**
 * Available Audible locales/regions
 * Each locale corresponds to a specific Audible marketplace
 */
const LOCALES: Locale[] = [
  { code: 'us', name: 'United States', flag: '🇺🇸' },
  { code: 'uk', name: 'United Kingdom', flag: '🇬🇧' },
  { code: 'de', name: 'Germany', flag: '🇩🇪' },
  { code: 'fr', name: 'France', flag: '🇫🇷' },
  { code: 'ca', name: 'Canada', flag: '🇨🇦' },
  { code: 'au', name: 'Australia', flag: '🇦🇺' },
  { code: 'it', name: 'Italy', flag: '🇮🇹' },
  { code: 'es', name: 'Spain', flag: '🇪🇸' },
  { code: 'in', name: 'India', flag: '🇮🇳' },
  { code: 'jp', name: 'Japan', flag: '🇯🇵' },
];

interface LocaleSelectorProps {
  selectedLocale: string;
  onSelect: (localeCode: string) => void;
}

/**
 * Locale selector component for choosing Audible region
 *
 * Displays a list of available Audible marketplaces with country flags.
 * The selected locale is highlighted and used for OAuth authentication.
 */
export default function LocaleSelector({ selectedLocale, onSelect }: LocaleSelectorProps) {
  return (
    <ScrollView style={styles.container} showsVerticalScrollIndicator={false}>
      <View style={styles.content}>
        <Text style={styles.title}>Select your Audible region</Text>
        <Text style={styles.subtitle}>
          Choose the region where your Audible account is registered
        </Text>

        <View style={styles.localeList}>
          {LOCALES.map(locale => (
            <TouchableOpacity
              key={locale.code}
              style={[
                styles.localeButton,
                selectedLocale === locale.code && styles.localeButtonSelected
              ]}
              onPress={() => onSelect(locale.code)}
              activeOpacity={0.7}
            >
              <View style={styles.localeContent}>
                <Text style={styles.flag}>{locale.flag}</Text>
                <Text style={[
                  styles.localeName,
                  selectedLocale === locale.code && styles.localeNameSelected
                ]}>
                  {locale.name}
                </Text>
              </View>
              {selectedLocale === locale.code && (
                <Text style={styles.checkmark}>✓</Text>
              )}
            </TouchableOpacity>
          ))}
        </View>
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  content: {
    padding: spacing.lg,
  },
  title: {
    ...typography.title,
    fontSize: 24,
    marginBottom: spacing.sm,
  },
  subtitle: {
    ...typography.caption,
    marginBottom: spacing.xl,
  },
  localeList: {
    gap: spacing.sm,
  },
  localeButton: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    backgroundColor: colors.backgroundSecondary,
    padding: spacing.md,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: colors.border,
  },
  localeButtonSelected: {
    backgroundColor: colors.accentDim,
    borderColor: colors.accent,
  },
  localeContent: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: spacing.md,
  },
  flag: {
    fontSize: 24,
  },
  localeName: {
    ...typography.body,
  },
  localeNameSelected: {
    fontWeight: '600',
  },
  checkmark: {
    fontSize: 20,
    color: colors.accent,
  },
});
