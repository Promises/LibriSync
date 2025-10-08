import React, { useRef, useState, useEffect } from 'react';
import { Modal, View, Text, StyleSheet, ActivityIndicator, TouchableOpacity, Alert } from 'react-native';
import { WebView, WebViewNavigation } from 'react-native-webview';
import { colors, spacing, typography } from '../styles/theme';
import type { TokenResponse, OAuthFlowData } from '../types/auth';

/**
 * Props for OAuthWebView component
 */
interface OAuthWebViewProps {
  visible: boolean;
  localeCode: string;
  onSuccess: (tokens: TokenResponse, deviceSerial: string) => void;
  onCancel: () => void;
  onError: (error: Error) => void;
}

/**
 * WebView-based OAuth authentication component for Audible
 *
 * This component handles the OAuth 2.0 + PKCE authentication flow:
 * 1. Generates OAuth URL with PKCE challenge
 * 2. Opens Amazon login in WebView
 * 3. Monitors for callback URL with authorization code
 * 4. Exchanges authorization code for access/refresh tokens
 * 5. Returns tokens to parent component via onSuccess callback
 */
export default function OAuthWebView({
  visible,
  localeCode,
  onSuccess,
  onCancel,
  onError
}: OAuthWebViewProps) {
  const [loading, setLoading] = useState(true);
  const [authUrl, setAuthUrl] = useState<string>('');
  const [flowData, setFlowData] = useState<OAuthFlowData | null>(null);
  const webViewRef = useRef<WebView>(null);

  /**
   * Initialize OAuth flow when component becomes visible
   * Generates device serial and fetches OAuth URL from Rust core
   */
  useEffect(() => {
    if (visible) {
      initializeOAuth();
    }
  }, [visible, localeCode]);

  /**
   * Initialize OAuth flow by generating device serial and OAuth URL
   */
  const initializeOAuth = async () => {
    try {
      setLoading(true);

      // Import Rust bridge
      const { initiateOAuth } = require('../../modules/expo-rust-bridge');

      // Initiate OAuth flow (generates device serial internally)
      const oauthData = initiateOAuth(localeCode);

      setAuthUrl(oauthData.url);
      setFlowData({
        deviceSerial: oauthData.deviceSerial,
        pkceVerifier: oauthData.pkceVerifier,
        localeCode
      });

      setLoading(false);
    } catch (error) {
      console.error('OAuth initialization error:', error);
      onError(error as Error);
    }
  };

  /**
   * Handle WebView navigation state changes
   * Monitors for callback URL and extracts authorization code
   */
  const handleNavigationStateChange = async (navState: WebViewNavigation) => {
    // Check if we've reached the callback URL
    if (navState.url.includes('/ap/maplanding') || navState.url.includes('openid.oa2.authorization_code=')) {
      setLoading(true);

      try {
        if (!flowData) {
          throw new Error('OAuth flow data not initialized');
        }

        // Import Rust bridge
        const { completeOAuthFlow } = require('../../modules/expo-rust-bridge');

        // Complete OAuth flow (parse callback, exchange code for tokens)
        const tokenResponse = await completeOAuthFlow(
          navState.url,
          flowData.localeCode,
          flowData.deviceSerial,
          flowData.pkceVerifier
        );

        // Pass tokens and device serial to parent
        onSuccess(tokenResponse, flowData.deviceSerial);

      } catch (error) {
        console.error('OAuth callback error:', error);
        onError(error as Error);
      } finally {
        setLoading(false);
      }
    }
  };

  return (
    <Modal
      visible={visible}
      animationType="slide"
      presentationStyle="pageSheet"
      onRequestClose={onCancel}
    >
      <View style={styles.container}>
        {/* Header */}
        <View style={styles.header}>
          <Text style={styles.headerTitle}>Sign in to Audible</Text>
          <TouchableOpacity onPress={onCancel} style={styles.cancelButton}>
            <Text style={styles.cancelButtonText}>Cancel</Text>
          </TouchableOpacity>
        </View>

        {/* WebView */}
        {loading && !authUrl ? (
          <View style={styles.loadingContainer}>
            <ActivityIndicator size="large" color={colors.accent} />
            <Text style={styles.loadingText}>Initializing authentication...</Text>
          </View>
        ) : (
          <>
            {loading && (
              <View style={styles.loadingOverlay}>
                <ActivityIndicator size="large" color={colors.accent} />
              </View>
            )}
            <WebView
              ref={webViewRef}
              source={{ uri: authUrl }}
              onNavigationStateChange={handleNavigationStateChange}
              onLoadStart={() => setLoading(true)}
              onLoadEnd={() => setLoading(false)}
              onError={(syntheticEvent) => {
                const { nativeEvent } = syntheticEvent;
                console.error('WebView error:', nativeEvent);
                onError(new Error(`WebView error: ${nativeEvent.description}`));
              }}
              style={styles.webview}
              // Enable JavaScript (required for Amazon login)
              javaScriptEnabled={true}
              // Enable DOM storage (required for some login flows)
              domStorageEnabled={true}
              // Start loading immediately
              startInLoadingState={true}
              // Allow third-party cookies (required for OAuth)
              thirdPartyCookiesEnabled={true}
              // iOS specific: Allow inline media playback
              allowsInlineMediaPlayback={true}
              // Android specific: Mixed content mode
              mixedContentMode="always"
            />
          </>
        )}
      </View>
    </Modal>
  );
}

/**
 * Generate a random device serial (32 hex characters)
 * This simulates a unique device identifier for Audible API
 */
function generateDeviceSerial(): string {
  const bytes = new Uint8Array(16);
  for (let i = 0; i < 16; i++) {
    bytes[i] = Math.floor(Math.random() * 256);
  }
  return Array.from(bytes)
    .map(b => b.toString(16).padStart(2, '0').toUpperCase())
    .join('');
}

/**
 * Generate a random string of specified length
 * Used for PKCE verifier generation
 */
function generateRandomString(length: number): string {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~';
  let result = '';
  for (let i = 0; i < length; i++) {
    result += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return result;
}

/**
 * Generate a mock OAuth URL for testing
 * TODO: Replace with actual Rust bridge call
 */
function generateMockOAuthUrl(localeCode: string): string {
  const domains: Record<string, string> = {
    us: 'amazon.com',
    uk: 'amazon.co.uk',
    de: 'amazon.de',
    fr: 'amazon.fr',
    ca: 'amazon.ca',
    au: 'amazon.com.au',
    it: 'amazon.it',
    es: 'amazon.es',
    in: 'amazon.in',
    jp: 'amazon.co.jp',
  };

  const domain = domains[localeCode] || 'amazon.com';

  // This is a mock URL - in production, this would be generated by Rust
  return `https://www.${domain}/ap/signin?openid.return_to=https://www.${domain}/ap/maplanding`;
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: colors.background,
  },
  header: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    padding: spacing.md,
    paddingTop: 60, // Account for status bar
    backgroundColor: colors.backgroundSecondary,
    borderBottomWidth: 1,
    borderBottomColor: colors.border,
  },
  headerTitle: {
    ...typography.subtitle,
  },
  cancelButton: {
    padding: spacing.sm,
  },
  cancelButtonText: {
    ...typography.body,
    color: colors.accent,
    fontWeight: '600',
  },
  loadingContainer: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    gap: spacing.md,
  },
  loadingOverlay: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    justifyContent: 'center',
    alignItems: 'center',
    backgroundColor: 'rgba(26, 26, 26, 0.8)',
    zIndex: 1,
  },
  loadingText: {
    ...typography.caption,
  },
  webview: {
    flex: 1,
    backgroundColor: colors.background,
  },
});
