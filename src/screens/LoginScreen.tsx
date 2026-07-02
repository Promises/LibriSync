import React, { useState, useRef } from 'react';
import { View, Text, Alert, ActivityIndicator, TouchableOpacity, ScrollView } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { WebView } from 'react-native-webview';
import {
  initiateOAuth,
  completeOAuthFlow,
  RustBridgeError,
} from '../../modules/expo-rust-bridge';
import type { Account, Locale } from '../../modules/expo-rust-bridge';
import { DEMO_ACCOUNT } from '../services/demo/demoData';
import { useStyles } from '../hooks/useStyles';
import { useTheme } from '../styles/theme';
import type { Theme } from '../hooks/useStyles';

interface LoginScreenProps {
  onLoginSuccess: (account: Account) => void;
  onCancel?: () => void;
  title?: string;
}

interface Region {
  code: string;
  name: string;
  domain: string;
  flag: string;
}

const REGIONS: Region[] = [
  { code: 'us', name: 'United States', domain: 'audible.com', flag: '🇺🇸' },
  { code: 'uk', name: 'United Kingdom', domain: 'audible.co.uk', flag: '🇬🇧' },
  { code: 'de', name: 'Germany', domain: 'audible.de', flag: '🇩🇪' },
  { code: 'fr', name: 'France', domain: 'audible.fr', flag: '🇫🇷' },
  { code: 'ca', name: 'Canada', domain: 'audible.ca', flag: '🇨🇦' },
  { code: 'au', name: 'Australia', domain: 'audible.com.au', flag: '🇦🇺' },
  { code: 'it', name: 'Italy', domain: 'audible.it', flag: '🇮🇹' },
  { code: 'es', name: 'Spain', domain: 'audible.es', flag: '🇪🇸' },
  { code: 'in', name: 'India', domain: 'audible.in', flag: '🇮🇳' },
  { code: 'jp', name: 'Japan', domain: 'audible.co.jp', flag: '🇯🇵' },
  { code: 'br', name: 'Brazil', domain: 'audible.com.br', flag: '🇧🇷' },
];

export default function LoginScreen({ onLoginSuccess, onCancel, title = 'Log in to Audible' }: LoginScreenProps) {
  const styles = useStyles(createStyles);
  const { colors, spacing } = useTheme();
  const [isLoading, setIsLoading] = useState(false);
  const [oauthUrl, setOauthUrl] = useState<string | null>(null);
  const [status, setStatus] = useState('Select your Audible region');
  const [selectedRegion, setSelectedRegion] = useState<Region | null>(null);

  const oauthDataRef = useRef<{
    pkceVerifier: string;
    state: string;
    deviceSerial: string;
    localeCode: string;
  } | null>(null);

  // Initiate OAuth flow
  const startOAuthFlow = async (region: Region) => {
    try {
      setIsLoading(true);
      setStatus('Generating OAuth URL...');
      console.log('[LoginScreen] Starting OAuth flow for region:', region.name, '(' + region.code + ')');

      const flowData = initiateOAuth(region.code);
      console.log('[LoginScreen] OAuth URL generated');

      oauthDataRef.current = {
        pkceVerifier: flowData.pkceVerifier,
        state: flowData.state,
        deviceSerial: flowData.deviceSerial,
        localeCode: region.code,
      };

      setOauthUrl(flowData.url);
      setStatus('Please log in with your Audible account');
      console.log('[LoginScreen] WebView should now load OAuth URL');
    } catch (error) {
      console.error('[LoginScreen] Failed to initiate OAuth:', error);
      Alert.alert(
        'OAuth Error',
        error instanceof RustBridgeError
          ? error.message
          : 'Failed to start login process'
      );
      setSelectedRegion(null);
      setOauthUrl(null);
    } finally {
      setIsLoading(false);
    }
  };

  const [isProcessingCallback, setIsProcessingCallback] = React.useState(false);

  // Handle WebView navigation state changes
  const handleNavigationStateChange = async (navState: any) => {
    const { url } = navState;
    console.log('[LoginScreen] WebView navigation event');

    // Log all maplanding URLs to debug OAuth flow
    if (url.includes('/ap/maplanding')) {
      console.log('[LoginScreen] Maplanding URL detected, checking for auth code...');
    }

    // Check if this is the callback URL with authorization code
    if (url.includes('/ap/maplanding') && (url.includes('openid.oa2.authorization_code=') || url.includes('?code=') || url.includes('&code='))) {
      console.log('[LoginScreen] Detected OAuth callback URL with authorization code');

      // Prevent processing the same callback twice
      if (isProcessingCallback) {
        console.log('[LoginScreen] Already processing callback, ignoring duplicate');
        return;
      }
      setIsProcessingCallback(true);
      try {
        if (!oauthDataRef.current) {
          throw new Error('OAuth data not found');
        }

        setStatus('Exchanging authorization code for tokens...');
        setIsLoading(true);
        console.log('[LoginScreen] Exchanging auth code for tokens...');

        // Complete OAuth flow
        const tokens = await completeOAuthFlow(
          url,
          oauthDataRef.current.localeCode,
          oauthDataRef.current.deviceSerial,
          oauthDataRef.current.pkceVerifier
        );
        console.log('[LoginScreen] Tokens received:', {
          hasAccessToken: !!tokens.bearer.access_token,
          hasRefreshToken: !!tokens.bearer.refresh_token,
          expiresIn: tokens.bearer.expires_in
        });

        setStatus('Retrieving activation bytes...');

        // Create account object with tokens
        const locale: Locale = {
          country_code: oauthDataRef.current.localeCode,
          name: getLocaleName(oauthDataRef.current.localeCode),
          domain: getLocaleDomain(oauthDataRef.current.localeCode),
          with_username: oauthDataRef.current.localeCode !== 'jp', // Japan uses phone, all others use email
        };

        // Parse expires_in (comes as string "3600" from API)
        const expiresInSeconds = parseInt(tokens.bearer.expires_in, 10);
        const expiresAt = new Date(Date.now() + expiresInSeconds * 1000);

        // Convert website cookies array to object
        const cookiesMap: Record<string, string> = {};
        tokens.website_cookies.forEach(cookie => {
          cookiesMap[cookie.Name] = cookie.Value;
        });

        const customerName = tokens.customer_info.name?.trim()
          || tokens.customer_info.given_name?.trim()
          || 'Audible Account';
        const regionCode = locale.country_code.toUpperCase();

        const account: Account = {
          account_id: `${tokens.customer_info.user_id}:${locale.country_code}`,
          account_name: `${customerName} (${regionCode})`,
          library_scan: true,
          decrypt_key: '',  // Will be filled by getActivationBytes
          locale,
          identity: {
            access_token: {
              token: tokens.bearer.access_token,
              expires_at: expiresAt.toISOString(),
            },
            refresh_token: tokens.bearer.refresh_token,
            device_private_key: tokens.mac_dms.device_private_key,
            adp_token: tokens.mac_dms.adp_token,
            cookies: cookiesMap,
            device_serial_number: tokens.device_info.device_serial_number,
            device_type: tokens.device_info.device_type,
            device_name: tokens.device_info.device_name,
            amazon_account_id: tokens.customer_info.user_id,
            store_authentication_cookie: tokens.store_authentication_cookie.cookie,
            locale,
            customer_info: tokens.customer_info,
          },
        };

        console.log('[LoginScreen] Skipping activation bytes during login');

        setStatus('Login successful!');
        console.log('[LoginScreen] Login complete! Calling onLoginSuccess');
        onLoginSuccess(account);
      } catch (error) {
        console.error('[LoginScreen] Failed to complete OAuth:', error);
        Alert.alert(
          'Authentication Error',
          error instanceof RustBridgeError
            ? error.message
            : 'Failed to complete login'
        );
        setIsLoading(false);
        setOauthUrl(null);
        setStatus('Login failed. Please try again.');
      }
    }
  };

  const handleRegionSelect = (region: Region) => {
    setSelectedRegion(region);
    startOAuthFlow(region);
  };

  // Demo mode: skip OAuth entirely and sign in with a fake Audible account
  // backed by free LibriVox books. Replicates the full Audible UI offline.
  const handleDemoMode = () => {
    console.log('[LoginScreen] Entering demo mode (no OAuth)');
    onLoginSuccess(DEMO_ACCOUNT);
  };

  const handleBackToRegionPicker = () => {
    console.log('[LoginScreen] User cancelled login, returning to region picker');
    setOauthUrl(null);
    setSelectedRegion(null);
    setIsProcessingCallback(false);
    setStatus('Select your Audible region');
  };

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      {oauthUrl ? (
        <>
          <View style={styles.webViewHeader}>
            <TouchableOpacity
              style={styles.backButton}
              onPress={handleBackToRegionPicker}
            >
              <Text style={styles.backButtonText}>← Back</Text>
            </TouchableOpacity>
            {selectedRegion && (
              <View style={styles.regionInfo}>
                <Text style={styles.regionInfoText}>
                  {selectedRegion.flag} {selectedRegion.name}
                </Text>
              </View>
            )}
          </View>
          <WebView
            source={{ uri: oauthUrl }}
            onNavigationStateChange={handleNavigationStateChange}
            style={styles.webView}
            onLoadStart={() => {
              console.log('[LoginScreen] WebView started loading');
              setIsLoading(true);
            }}
            onLoadEnd={() => {
              console.log('[LoginScreen] WebView finished loading');
              setIsLoading(false);
            }}
          />
          {isLoading && (
            <View style={styles.loadingOverlay}>
              <ActivityIndicator size="large" color={colors.accent} />
              <Text style={styles.statusText}>{status}</Text>
            </View>
          )}
        </>
      ) : (
        <ScrollView contentContainerStyle={styles.regionPickerContainer}>
          <Text style={styles.title}>{title}</Text>
          <Text style={styles.subtitle}>Select your region to continue</Text>

          {onCancel && (
            <TouchableOpacity
              style={styles.cancelButton}
              onPress={onCancel}
              disabled={isLoading}
            >
              <Text style={styles.cancelButtonText}>Back to Accounts</Text>
            </TouchableOpacity>
          )}

          <View style={styles.regionGrid}>
            <TouchableOpacity
              style={[styles.regionCard, styles.demoCard]}
              onPress={handleDemoMode}
              disabled={isLoading}
            >
              <Text style={styles.regionName}>🎬 Demo Mode</Text>
              <Text style={styles.regionDomain}>no login</Text>
            </TouchableOpacity>
            {REGIONS.map((region) => (
              <TouchableOpacity
                key={region.code}
                style={styles.regionCard}
                onPress={() => handleRegionSelect(region)}
                disabled={isLoading}
              >
                <Text style={styles.regionName}>{region.flag} {region.name}</Text>
                <Text style={styles.regionDomain}>{region.domain}</Text>
              </TouchableOpacity>
            ))}
          </View>

          {isLoading && (
            <View style={styles.loadingContainer}>
              <ActivityIndicator size="large" color={colors.accent} />
              <Text style={styles.statusText}>{status}</Text>
            </View>
          )}
        </ScrollView>
      )}
    </SafeAreaView>
  );
}

// Helper functions for locale info
function getLocaleName(code: string): string {
  const region = REGIONS.find(r => r.code === code);
  return region ? region.name : code.toUpperCase();
}

function getLocaleDomain(code: string): string {
  const region = REGIONS.find(r => r.code === code);
  return region ? region.domain : 'audible.com';
}

const createStyles = (theme: Theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  webViewHeader: {
    flexDirection: 'row' as const,
    alignItems: 'center' as const,
    padding: theme.spacing.md,
    backgroundColor: theme.colors.backgroundSecondary,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  backButton: {
    padding: theme.spacing.sm,
    marginRight: theme.spacing.md,
  },
  backButtonText: {
    ...theme.typography.body,
    color: theme.colors.accent,
    fontWeight: '600' as const,
  },
  regionInfo: {
    flex: 1,
    alignItems: 'center' as const,
  },
  regionInfoText: {
    ...theme.typography.body,
    fontWeight: '600' as const,
  },
  webView: {
    flex: 1,
  },
  regionPickerContainer: {
    flexGrow: 1,
    padding: theme.spacing.lg,
  },
  title: {
    ...theme.typography.title,
    marginBottom: theme.spacing.sm,
    textAlign: 'center' as const,
  },
  subtitle: {
    ...theme.typography.body,
    textAlign: 'center' as const,
    marginBottom: theme.spacing.xl,
    color: theme.colors.textSecondary,
  },
  // Same shape as a region card, with a subtle accent border to hint it's special.
  demoCard: {
    borderColor: theme.colors.accent,
  },
  regionGrid: {
    gap: theme.spacing.md,
  },
  cancelButton: {
    padding: theme.spacing.md,
    marginBottom: theme.spacing.md,
    alignItems: 'center' as const,
  },
  cancelButtonText: {
    ...theme.typography.body,
    color: theme.colors.accent,
    fontWeight: '600' as const,
  },
  regionCard: {
    backgroundColor: theme.colors.backgroundSecondary,
    padding: theme.spacing.lg,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: theme.colors.border,
    alignItems: 'center' as const,
    gap: theme.spacing.xs,
  },
  // regionFlag: {
  //   fontSize: 40,
  //   marginBottom: theme.spacing.xs,
  // },
  regionName: {
    ...theme.typography.body,
    fontWeight: '600' as const,
    textAlign: 'center' as const,
  },
  regionDomain: {
    ...theme.typography.caption,
    color: theme.colors.textSecondary,
  },
  loadingContainer: {
    marginTop: theme.spacing.xl,
    alignItems: 'center' as const,
  },
  loadingOverlay: {
    position: 'absolute' as const,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    backgroundColor: `${theme.colors.background}E6`, // 90% opacity
    justifyContent: 'center' as const,
    alignItems: 'center' as const,
    zIndex: 1000,
  },
  statusText: {
    ...theme.typography.body,
    marginTop: theme.spacing.md,
    textAlign: 'center' as const,
  },
});
