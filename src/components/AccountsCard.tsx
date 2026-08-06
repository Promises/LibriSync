import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import Button from './Button';
import { useStyles } from '../hooks/useStyles';
import type { Theme } from '../hooks/useStyles';
import type { Account } from '../../modules/expo-rust-bridge';

interface Props {
  /** Card heading, e.g. "Audible Accounts". */
  label: string;
  accounts: Account[];
  selectedAccountId: string | null;
  onSelect: (account: Account) => void;
  onAddAccount: () => void;
  /** Row title, e.g. "Henning Berge (US)". */
  formatName: (account: Account) => string;
  /** Row subtitle — region for Audible, email for a credential provider. */
  formatSubtitle?: (account: Account) => string | null;
  addAccountTitle?: string;
  disabled?: boolean;
}

/**
 * The list of a single provider's accounts, with an Add Account action.
 *
 * Provider-agnostic on purpose: pass an already-scoped `accounts` array (see
 * `getAllAccounts(dbPath, provider)`) plus formatters for whatever that provider
 * shows per row. A new provider reuses this as-is.
 */
export default function AccountsCard({
  label,
  accounts,
  selectedAccountId,
  onSelect,
  onAddAccount,
  formatName,
  formatSubtitle,
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
        return (
          <TouchableOpacity
            key={account.account_id}
            style={[styles.accountRow, isSelected && styles.accountRowSelected]}
            onPress={() => onSelect(account)}
            disabled={isSelected || disabled}
          >
            <View style={styles.accountText}>
              <Text style={styles.value}>{formatName(account)}</Text>
              {!!subtitle && <Text style={styles.caption}>{subtitle}</Text>}
            </View>
            {isSelected && <Text style={styles.selectedMark}>✓</Text>}
          </TouchableOpacity>
        );
      })}
      <Button
        title={addAccountTitle}
        onPress={onAddAccount}
        variant="outlined"
        state="primary"
        disabled={disabled}
        style={{ marginTop: 8 }}
      />
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
