import React, { createContext, useContext, useState, useEffect, useCallback } from 'react';
import * as SecureStore from 'expo-secure-store';
import type { ProviderId } from '../../modules/expo-rust-bridge';

const PROVIDERS_KEY = 'enabled_providers';

/**
 * Which stores the app offers: in the Add Account picker, and LibriVox's Browse button.
 * Turning one off doesn't touch accounts already added or books already downloaded.
 */
export type EnabledProviders = Record<ProviderId, boolean>;

interface ProvidersContextValue {
  providers: EnabledProviders;
  setProvider: (provider: ProviderId, enabled: boolean) => void;
}

const defaultProviders: EnabledProviders = { audible: true, librivox: true, librofm: true, chirp: true };

const ProvidersContext = createContext<ProvidersContextValue>({
  providers: defaultProviders,
  setProvider: () => {},
});

export function ProvidersProvider({ children }: { children: React.ReactNode }) {
  const [providers, setProviders] = useState<EnabledProviders>(defaultProviders);

  useEffect(() => {
    SecureStore.getItemAsync(PROVIDERS_KEY).then(value => {
      if (value) {
        try {
          // Saved before a store existed: it is missing from the JSON and starts enabled.
          setProviders({ ...defaultProviders, ...JSON.parse(value) });
        } catch {}
      }
    });
  }, []);

  const setProvider = useCallback((provider: ProviderId, enabled: boolean) => {
    setProviders(prev => {
      const next = { ...prev, [provider]: enabled };
      // Keep at least one store on, or the app would have nothing to offer.
      if (!Object.values(next).some(Boolean)) return prev;

      SecureStore.setItemAsync(PROVIDERS_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  return (
    <ProvidersContext.Provider value={{ providers, setProvider }}>
      {children}
    </ProvidersContext.Provider>
  );
}

export function useProviders() {
  return useContext(ProvidersContext);
}
