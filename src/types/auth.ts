/**
 * OAuth and Authentication Types
 */

export interface TokenResponse {
  access_token: string;
  refresh_token: string;
  expires_in: number;
  token_type: string;
}

export interface OAuthFlowData {
  deviceSerial: string;
  pkceVerifier: string;
  localeCode: string;
}

export interface OAuthUrlResult {
  success: boolean;
  data?: {
    url: string;
    pkce_verifier: string;
  };
  error?: string;
}

export interface OAuthCallbackResult {
  success: boolean;
  data?: {
    authorization_code: string;
  };
  error?: string;
}

export interface TokenExchangeResult {
  success: boolean;
  data?: TokenResponse;
  error?: string;
}

export interface Locale {
  code: string;
  name: string;
  flag: string;
}
