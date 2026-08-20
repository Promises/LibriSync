import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import Button from './Button';
import { useStyles } from '../hooks/useStyles';
import type { Theme } from '../hooks/useStyles';
import type { Account } from '../../modules/expo-rust-bridge';

interface Props {
  /** Card heading, e.g. "Audible Accounts" or just "Accounts" when mixed. */
  label: string;
  accounts: Account[];
  selectedAccountId: string | null;
  onSelect: (account: Account) => void;
  /** Omit to hide the Add Account button — e.g. when the screen owns that flow. */
  onAddAccount?: () => void;
  /** Row title, e.g. "Henning Berge (US)". */
  formatName: (account: Account) => string;
  /** Row subtitle — region for Audible, email for a credential provider. */
  formatSubtitle?: (account: Account) => string | null;
  /** Leading badge per row. Needed once the list mixes providers. */
  formatIcon?: (account: Account) => { name: React.ComponentProps<typeof Ionicons>['name']; color: string };
  addAccountTitle?: string;
  disabled?: boolean;
}

/**
 * A list of accounts with an optional Add Account action.
 *
 * Provider-agnostic on purpose: pass whatever `accounts` array you want shown —
 * one provider's (see `getAllAccounts(dbPath, provider)`) or every provider's,
 * in which case supply `formatIcon` so rows are attributable.
 */
export default function AccountsCard({
  label,
  accounts,
  selectedAccountId,
  onSelect,
  onAddAccount,
  formatName,
  formatSubtitle,
  formatIcon,
  addAccountTitle = 'Add Account',
  disabled = false,
}: Props) {
  const styles = useStyles(createStyles);

  return (
    <View style={styles.card}>
      <Text style={styles.label}>{label}</Text>
      {accounts.map((account) => {
        const isSelected = account.account_id === selectedAccountId;
        const subtitle = formatSubtitle?.(account);
        const icon = formatIcon?.(account);
        return (
          <TouchableOpacity
            key={account.account_id}
            style={[styles.accountRow, isSelected && styles.accountRowSelected]}
            onPress={() => onSelect(account)}
            disabled={isSelected || disabled}
          >
            {!!icon && (
              <View style={styles.accountIcon}>
                <Ionicons name={icon.name} size={20} color={icon.color} />
              </View>
            )}
            <View style={styles.accountText}>
              <Text style={styles.value}>{formatName(account)}</Text>
              {!!subtitle && <Text style={styles.caption}>{subtitle}</Text>}
            </View>
            {isSelected && <Text style={styles.selectedMark}>✓</Text>}
          </TouchableOpacity>
        );
      })}
      {!!onAddAccount && (
        <Button
          title={addAccountTitle}
          onPress={onAddAccount}
          variant="outlined"
          state="primary"
          disabled={disabled}
          style={{ marginTop: 8 }}
        />
      )}
    </View>
  );
}

const createStyles = (theme: Theme) => ({
  card: {
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.md,
    borderRadius: 8,
    marginBottom: theme.spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  label: {
    ...theme.typography.caption,
    marginBottom: theme.spacing.xs,
    textTransform: 'uppercase' as const,
  },
  accountRow: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    justifyContent: 'space-between' as const,
    paddingVertical: theme.spacing.sm,
    paddingHorizontal: theme.spacing.sm,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: 'transparent',
  },
  accountRowSelected: {
    borderColor: theme.colors.accent,
    backgroundColor: theme.colors.accentDim,
  },
  accountIcon: {
    width: 32,
    height: 32,
    borderRadius: 8,
    backgroundColor: theme.colors.backgroundTertiary,
    justifyContent: 'center' as const,
    alignItems: 'center' as const,
    marginRight: theme.spacing.sm,
  },
  accountText: {
    flex: 1,
    paddingRight: theme.spacing.sm,
  },
  value: {
    ...theme.typography.body,
  },
  caption: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
  },
  selectedMark: {
    ...theme.typography.body,
    color: theme.colors.accent,
    fontWeight: '700' as const,
  },
});
