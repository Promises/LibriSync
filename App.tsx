import { StatusBar } from 'expo-status-bar';
import { useColorScheme } from 'react-native';
import { useEffect } from 'react';
import AppNavigator from './src/navigation/AppNavigator';
import ErrorBoundary from './src/components/ErrorBoundary';
import { ProvidersProvider } from './src/contexts/ProvidersContext';
import { refreshClientProfiles, startBackgroundService } from './modules/expo-rust-bridge';
import { getDatabasePath } from './src/utils/appPaths';

export default function App() {
  const colorScheme = useColorScheme();

  // Start background service on app launch
  useEffect(() => {
    try {
      console.log('[App] Starting background service...');
      startBackgroundService();
      console.log('[App] Background service started successfully');
    } catch (error) {
      console.error('[App] Failed to start background service:', error);
    }
  }, []);

  // Pick up store User-Agent changes published since this build (at most daily).
  // Best effort: offline or a bad file leaves the current values in place.
  useEffect(() => {
    refreshClientProfiles(getDatabasePath())
      .then(({ fetched, changed, profiles }) => {
        if (fetched) {
          console.log(`[App] Client profiles ${changed ? 'updated' : 'unchanged'} (issued ${profiles.issued})`);
        }
      })
      .catch((error) => console.warn('[App] Client profiles check failed:', error));
  }, []);

  return (
    <ErrorBoundary>
      <ProvidersProvider>
        <AppNavigator />
        <StatusBar style={colorScheme === 'dark' ? 'light' : 'dark'} />
      </ProvidersProvider>
    </ErrorBoundary>
  );
}
