import React, { useState, useRef } from 'react';
import { View, Text, StyleSheet, Alert, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { WebView } from 'react-native-webview';
import {
  initiateOAuth,
  completeOAuthFlow,
  getActivationBytes,
  RustBridgeError,
} from '../../modules/expo-rust-bridge';
import type { Account, Locale } from '../../modules/expo-rust-bridge';
import * as SecureStore from 'expo-secure-store';

interface LoginScreenProps {
  onLoginSuccess: (account: Account) => void;
}

export default function LoginScreen({ onLoginSuccess }: LoginScreenProps) {
  const [isLoading, setIsLoading] = useState(false);
  const [oauthUrl, setOauthUrl] = useState<string | null>(null);
  const [status, setStatus] = useState('Preparing login...');

  const oauthDataRef = useRef<{
    pkceVerifier: string;
    state: string;
    deviceSerial: string;
    localeCode: string;
  } | null>(null);

  // Initiate OAuth flow
  const startOAuthFlow = async (localeCode: string = 'us') => {
    try {
      setIsLoading(true);
      setStatus('Generating OAuth URL...');
      console.log('[LoginScreen] Starting OAuth flow for locale:', localeCode);

      const flowData = initiateOAuth(localeCode);
      console.log('[LoginScreen] OAuth URL generated:', flowData.url);
      console.log('[LoginScreen] Device serial:', flowData.deviceSerial);

      oauthDataRef.current = {
        pkceVerifier: flowData.pkceVerifier,
        state: flowData.state,
        deviceSerial: flowData.deviceSerial,
        localeCode,
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
    } finally {
      setIsLoading(false);
    }
  };

  const [isProcessingCallback, setIsProcessingCallback] = React.useState(false);

  // Handle WebView navigation state changes
  const handleNavigationStateChange = async (navState: any) => {
    const { url } = navState;
    console.log('[LoginScreen] WebView navigated to:', url);

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

        const account: Account = {
          account_id: tokens.customer_info.user_id,
          account_name: tokens.customer_info.name,
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

        // Get activation bytes
        try {
          console.log('[LoginScreen] Requesting activation bytes...');
          const activationBytes = await getActivationBytes(account);
          account.decrypt_key = activationBytes;
          console.log('[LoginScreen] Activation bytes received:', activationBytes);
        } catch (error) {
          console.warn('[LoginScreen] Failed to get activation bytes:', error);
          // Continue without activation bytes - can get them later
        }

        // Store account in secure storage
        console.log('[LoginScreen] Storing account in secure storage...');
        await SecureStore.setItemAsync('audible_account', JSON.stringify(account));

        // Store token expiry if available
        if (account.identity?.access_token?.expires_at) {
          console.log('[LoginScreen] Storing token expiry:', account.identity.access_token.expires_at);
          await SecureStore.setItemAsync('token_expires_at', account.identity.access_token.expires_at);
        }

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

  // Start OAuth flow on mount
  React.useEffect(() => {
    console.log('[LoginScreen] Component mounted, starting OAuth flow');
    startOAuthFlow();
  }, []);

  return (
    <SafeAreaView style={styles.container} edges={['top', 'left', 'right']}>
      {oauthUrl ? (
        <>
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
              <ActivityIndicator size="large" color="#ff9500" />
              <Text style={styles.statusText}>{status}</Text>
            </View>
          )}
        </>
      ) : (
        <View style={styles.centered}>
          <ActivityIndicator size="large" color="#ff9500" />
          <Text style={styles.statusText}>{status}</Text>
        </View>
      )}
    </SafeAreaView>
  );
}

// Helper functions for locale info
function getLocaleName(code: string): string {
  const names: Record<string, string> = {
    us: 'United States',
    uk: 'United Kingdom',
    de: 'Germany',
    fr: 'France',
    ca: 'Canada',
    au: 'Australia',
    it: 'Italy',
    es: 'Spain',
    in: 'India',
    jp: 'Japan',
  };
  return names[code] || code.toUpperCase();
}

function getLocaleDomain(code: string): string {
  const domains: Record<string, string> = {
    us: 'audible.com',
    uk: 'audible.co.uk',
    de: 'audible.de',
    fr: 'audible.fr',
    ca: 'audible.ca',
    au: 'audible.com.au',
    it: 'audible.it',
    es: 'audible.es',
    in: 'audible.in',
    jp: 'audible.co.jp',
  };
  return domains[code] || 'audible.com';
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#1a1a1a',
  },
  webView: {
    flex: 1,
  },
  centered: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    padding: 20,
  },
  loadingOverlay: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    backgroundColor: 'rgba(26, 26, 26, 0.9)',
    justifyContent: 'center',
    alignItems: 'center',
    zIndex: 1000,
  },
  statusText: {
    color: '#ffffff',
    fontSize: 16,
    marginTop: 16,
    textAlign: 'center',
  },
});
