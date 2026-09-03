import React, { useCallback, useEffect, useState } from 'react';
import { View, Text, Alert } from 'react-native';
import * as SecureStore from 'expo-secure-store';
import Button from './Button';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';
import { getCustomerInformation } from '../../modules/expo-rust-bridge';
import type { Account } from '../../modules/expo-rust-bridge';
import { getDatabasePath } from '../utils/appPaths';
import { refreshAccountToken } from '../services/providerAccounts';
import { formatAudibleAccountRegion, getLocaleCode } from '../services/providers/audible';
import { isDemoAccount } from '../services/demo/demoMode';

interface Props {
  account: Account;
  /** Called with a token-refreshed copy so the screen's account list stays current. */
  onAccountUpdated: (account: Account) => void;
}

type ConnectionStatus = 'connected' | 'error' | 'checking';

function formatTimeRemaining(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

/**
 * The cards only Audible has: customer name, live connection check, region, and
 * the expiring access token with a manual refresh.
 *
 * Owns its own state keyed off `account` so the unified Accounts screen can just
 * render it for whichever account is selected without knowing any of this.
 */
export default function AudibleAccountDetails({ account, onAccountUpdated }: Props) {
  const styles = useStyles(createStyles);
  const { colors, spacing } = useTheme();

  const [accountName, setAccountName] = useState<string | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('checking');
  const [tokenExpiry, setTokenExpiry] = useState<Date | null>(null);
  const [timeRemaining, setTimeRemaining] = useState<number | null>(null);
  const [isRefreshingToken, setIsRefreshingToken] = useState(false);

  const updateTimeRemaining = (expiry: Date) => {
    setTimeRemaining(Math.max(0, Math.floor((expiry.getTime() - Date.now()) / 1000)));
  };

  const testConnection = useCallback(async () => {
    if (!account.identity) return;

    // Demo mode: no Audible API call — report a healthy connection.
    if (isDemoAccount(account)) {
      setAccountName(account.account_name ?? null);
      setConnectionStatus('connected');
      return;
    }

    try {
      setConnectionStatus('checking');
      const accessToken = typeof account.identity.access_token === 'string'
        ? account.identity.access_token
        : account.identity.access_token.token;
      const localeCode = getLocaleCode(account);

      if (!localeCode) throw new Error('Missing Audible region');

      const customerInfo = await getCustomerInformation(localeCode, accessToken);
      setAccountName(customerInfo.name || account.identity.customer_info?.name || null);
      setConnectionStatus('connected');
    } catch (error: any) {
      // Customer info is a *signed* request (device key from registration). Accounts
      // added before the app stored those credentials cannot make it — that is not a
      // connection problem, and reporting it as one is worse than saying nothing.
      const message: string = error?.rustError || error?.message || '';
      const missingDeviceCredentials = message.includes('ADP token')
        || message.includes('device private key');

      if (missingDeviceCredentials) {
        console.warn('[AudibleAccountDetails] Account predates device-key storage; sign in again for full details');
        setAccountName(account.identity?.customer_info?.name || account.account_name || null);
        setConnectionStatus('connected');
        return;
      }

      console.error('[AudibleAccountDetails] Failed to fetch customer info:', error);
      setAccountName(account.identity?.customer_info?.name || null);
      setConnectionStatus('error');
    }
  }, [account]);

  useEffect(() => {
    const expiresAt = account.identity?.access_token?.expires_at;
    if (expiresAt) {
      const expiry = new Date(expiresAt);
      setTokenExpiry(expiry);
      updateTimeRemaining(expiry);
      SecureStore.setItemAsync('token_expires_at', expiresAt).catch(() => {});
    } else {
      setTokenExpiry(null);
      setTimeRemaining(null);
    }

    testConnection();
  }, [account, testConnection]);

  useEffect(() => {
    if (!tokenExpiry) return;
    const interval = setInterval(() => updateTimeRemaining(tokenExpiry), 60000);
    return () => clearInterval(interval);
  }, [tokenExpiry]);

  const handleRefreshToken = async () => {
    if (!account.identity) {
      Alert.alert('Error', 'No authentication data available');
      return;
    }

    if (isDemoAccount(account)) {
      Alert.alert('Demo Mode', 'This is a demo account — no Audible token to refresh.');
      return;
    }

    try {
      setIsRefreshingToken(true);
      const refreshed = await refreshAccountToken(getDatabasePath(), account);
      const expiresAt = refreshed.identity?.access_token?.expires_at;

      if (expiresAt) {
        await SecureStore.setItemAsync('token_expires_at', expiresAt);
        const expiry = new Date(expiresAt);
        setTokenExpiry(expiry);
        updateTimeRemaining(expiry);
      }

      onAccountUpdated(refreshed);
      Alert.alert('Success', 'Access token refreshed successfully');
    } catch (error: any) {
      console.error('[AudibleAccountDetails] Token refresh failed:', error);
      Alert.alert('Error', error?.message || 'Failed to refresh token');
    } finally {
      setIsRefreshingToken(false);
    }
  };

  const statusColor = connectionStatus === 'connected'
    ? colors.success
    : connectionStatus === 'error'
      ? colors.error
      : colors.textSecondary;

  return (
    <>
      {!!accountName && (
        <View style={styles.card}>
          <Text style={styles.label}>Name</Text>
          <Text style={styles.value}>{accountName}</Text>
          {!!account.identity?.customer_info?.user_id && (
            <Text style={styles.caption}>
              ID: {account.identity.customer_info.user_id.substring(0, 30)}...
            </Text>
          )}
        </View>
      )}

      <View style={styles.card}>
        <Text style={styles.label}>Connection Status</Text>
        <View style={styles.statusRow}>
          <View style={[styles.statusIndicator, { backgroundColor: statusColor }]} />
          <Text style={styles.value}>
            {connectionStatus === 'connected'
              ? 'Connected'
              : connectionStatus === 'error'
                ? 'Connection Error'
                : 'Checking...'}
          </Text>
        </View>
      </View>

      <View style={styles.card}>
        <Text style={styles.label}>Region</Text>
        <Text style={styles.value}>{formatAudibleAccountRegion(account)}</Text>
      </View>

      {!!tokenExpiry && (
        <View style={styles.card}>
          <Text style={styles.label}>Access Token</Text>
          <Text style={styles.value}>{tokenExpiry < new Date() ? 'Expired' : 'Active'}</Text>
          <Text style={styles.caption}>Expires: {tokenExpiry.toLocaleString()}</Text>
          {timeRemaining !== null && timeRemaining > 0 && (
            <Text style={styles.caption}>
              Time remaining: {formatTimeRemaining(timeRemaining)}
            </Text>
          )}
          <Button
            title="Refresh Token"
            onPress={handleRefreshToken}
            variant="outlined"
            state="primary"
            loading={isRefreshingToken}
            style={{ marginTop: spacing.sm }}
          />
        </View>
      )}
    </>
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
  value: {
    ...theme.typography.body,
    fontWeight: '600' as const,
  },
  caption: {
    ...theme.typography.caption,
    marginTop: theme.spacing.xs,
  },
  statusRow: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
  },
  statusIndicator: {
    width: 8,
    height: 8,
    borderRadius: 4,
    marginRight: theme.spacing.sm,
  },
});
