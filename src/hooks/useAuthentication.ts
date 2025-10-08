import { useState, useEffect } from 'react';
import * as SecureStore from 'expo-secure-store';
import type { TokenResponse } from '../types/auth';

/**
 * Storage keys for secure token storage
 */
const STORAGE_KEYS = {
  ACCESS_TOKEN: 'audible_access_token',
  REFRESH_TOKEN: 'audible_refresh_token',
  TOKEN_EXPIRES_AT: 'audible_token_expires_at',
  DEVICE_SERIAL: 'audible_device_serial',
  LOCALE_CODE: 'audible_locale_code',
};

/**
 * Authentication state and methods
 */
export interface AuthState {
  isAuthenticated: boolean;
  isLoading: boolean;
  error: Error | null;
  tokens: TokenResponse | null;
  localeCode: string | null;
}

/**
 * Authentication hook for managing Audible OAuth tokens
 *
 * Provides methods for:
 * - Storing tokens securely
 * - Checking authentication status
 * - Logging out
 * - Refreshing tokens (TODO)
 *
 * Tokens are stored using Expo SecureStore for platform-native secure storage
 * (iOS Keychain, Android Keystore)
 */
export function useAuthentication() {
  const [state, setState] = useState<AuthState>({
    isAuthenticated: false,
    isLoading: true,
    error: null,
    tokens: null,
    localeCode: null,
  });

  /**
   * Check authentication status on mount
   */
  useEffect(() => {
    checkAuthentication();
  }, []);

  /**
   * Handle successful OAuth authentication
   * Stores tokens securely and updates state
   */
  const handleOAuthSuccess = async (
    tokenResponse: TokenResponse,
    localeCode: string,
    deviceSerial?: string
  ) => {
    try {
      // Store tokens securely
      await SecureStore.setItemAsync(STORAGE_KEYS.ACCESS_TOKEN, tokenResponse.access_token);
      await SecureStore.setItemAsync(STORAGE_KEYS.REFRESH_TOKEN, tokenResponse.refresh_token);
      await SecureStore.setItemAsync(
        STORAGE_KEYS.TOKEN_EXPIRES_AT,
        new Date(Date.now() + tokenResponse.expires_in * 1000).toISOString()
      );
      await SecureStore.setItemAsync(STORAGE_KEYS.LOCALE_CODE, localeCode);

      // Store device serial if provided (from initial OAuth flow)
      if (deviceSerial) {
        await SecureStore.setItemAsync(STORAGE_KEYS.DEVICE_SERIAL, deviceSerial);
      }

      setState({
        isAuthenticated: true,
        isLoading: false,
        error: null,
        tokens: tokenResponse,
        localeCode,
      });
    } catch (error) {
      console.error('Failed to store tokens:', error);
      setState(prev => ({
        ...prev,
        isLoading: false,
        error: error as Error,
      }));
    }
  };

  /**
   * Log out user and clear stored tokens
   */
  const logout = async () => {
    try {
      // Clear all stored authentication data
      await Promise.all([
        SecureStore.deleteItemAsync(STORAGE_KEYS.ACCESS_TOKEN),
        SecureStore.deleteItemAsync(STORAGE_KEYS.REFRESH_TOKEN),
        SecureStore.deleteItemAsync(STORAGE_KEYS.TOKEN_EXPIRES_AT),
        SecureStore.deleteItemAsync(STORAGE_KEYS.DEVICE_SERIAL),
        SecureStore.deleteItemAsync(STORAGE_KEYS.LOCALE_CODE),
      ]);

      setState({
        isAuthenticated: false,
        isLoading: false,
        error: null,
        tokens: null,
        localeCode: null,
      });
    } catch (error) {
      console.error('Failed to logout:', error);
      setState(prev => ({
        ...prev,
        error: error as Error,
      }));
    }
  };

  /**
   * Check if user is authenticated
   * Validates stored tokens and expiration
   */
  const checkAuthentication = async () => {
    try {
      setState(prev => ({ ...prev, isLoading: true }));

      const [accessToken, refreshToken, expiresAt, localeCode] = await Promise.all([
        SecureStore.getItemAsync(STORAGE_KEYS.ACCESS_TOKEN),
        SecureStore.getItemAsync(STORAGE_KEYS.REFRESH_TOKEN),
        SecureStore.getItemAsync(STORAGE_KEYS.TOKEN_EXPIRES_AT),
        SecureStore.getItemAsync(STORAGE_KEYS.LOCALE_CODE),
      ]);

      if (!accessToken || !refreshToken) {
        setState({
          isAuthenticated: false,
          isLoading: false,
          error: null,
          tokens: null,
          localeCode: null,
        });
        return false;
      }

      // Check if token is expired or expiring soon (within 5 minutes)
      const expiresDate = expiresAt ? new Date(expiresAt) : null;
      const now = new Date();
      const fiveMinutesFromNow = new Date(now.getTime() + 5 * 60 * 1000);

      const isExpired = expiresDate ? expiresDate < now : false;
      const isExpiringSoon = expiresDate ? expiresDate < fiveMinutesFromNow : false;

      if (isExpired || isExpiringSoon) {
        console.log('Token expired or expiring soon, attempting refresh...');
        const refreshSuccess = await refreshAccessToken();
        if (!refreshSuccess) {
          console.warn('Token refresh failed, logging out');
          await logout();
          return false;
        }
        return true;
      }

      // Token is valid
      setState({
        isAuthenticated: true,
        isLoading: false,
        error: null,
        tokens: {
          access_token: accessToken,
          refresh_token: refreshToken,
          expires_in: expiresAt
            ? Math.floor((new Date(expiresAt).getTime() - Date.now()) / 1000)
            : 3600,
          token_type: 'Bearer',
        },
        localeCode,
      });
      return true;
    } catch (error) {
      console.error('Failed to check authentication:', error);
      setState({
        isAuthenticated: false,
        isLoading: false,
        error: error as Error,
        tokens: null,
        localeCode: null,
      });
      return false;
    }
  };

  /**
   * Refresh access token using refresh token
   */
  const refreshAccessToken = async (): Promise<boolean> => {
    try {
      setState(prev => ({ ...prev, isLoading: true }));

      const refreshTokenValue = await SecureStore.getItemAsync(STORAGE_KEYS.REFRESH_TOKEN);
      const localeCode = await SecureStore.getItemAsync(STORAGE_KEYS.LOCALE_CODE);
      const deviceSerial = await SecureStore.getItemAsync(STORAGE_KEYS.DEVICE_SERIAL);

      if (!refreshTokenValue || !localeCode || !deviceSerial) {
        throw new Error('Missing refresh token, locale code, or device serial');
      }

      // Call Rust bridge to refresh token
      const ExpoRustBridge = require('../../modules/expo-rust-bridge').default;
      const result = await ExpoRustBridge.refreshAccessToken(
        localeCode,
        refreshTokenValue,
        deviceSerial
      );

      if (!result.success || !result.data) {
        throw new Error(result.error || 'Failed to refresh token');
      }

      // Update token storage with new tokens
      const tokenResponse: TokenResponse = {
        access_token: result.data.access_token,
        refresh_token: result.data.refresh_token || refreshTokenValue,
        expires_in: result.data.expires_in,
        token_type: result.data.token_type || 'Bearer',
      };

      await handleOAuthSuccess(tokenResponse, localeCode);
      console.log('Token refreshed successfully');
      return true;
    } catch (error) {
      console.error('Failed to refresh token:', error);
      setState(prev => ({
        ...prev,
        isLoading: false,
        error: error as Error,
      }));
      return false;
    }
  };

  /**
   * Get token expiration date
   */
  const getTokenExpiryDate = async (): Promise<Date | null> => {
    try {
      const expiresAt = await SecureStore.getItemAsync(STORAGE_KEYS.TOKEN_EXPIRES_AT);
      return expiresAt ? new Date(expiresAt) : null;
    } catch (error) {
      console.error('Failed to get token expiry date:', error);
      return null;
    }
  };

  /**
   * Get time remaining until token expiry in seconds
   */
  const getTimeUntilExpiry = async (): Promise<number | null> => {
    try {
      const expiryDate = await getTokenExpiryDate();
      if (!expiryDate) return null;

      const secondsRemaining = Math.floor((expiryDate.getTime() - Date.now()) / 1000);
      return Math.max(0, secondsRemaining);
    } catch (error) {
      console.error('Failed to get time until expiry:', error);
      return null;
    }
  };

  return {
    ...state,
    handleOAuthSuccess,
    logout,
    checkAuthentication,
    refreshToken: refreshAccessToken,
    getTokenExpiryDate,
    getTimeUntilExpiry,
  };
}
