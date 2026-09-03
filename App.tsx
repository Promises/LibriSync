import { StatusBar } from 'expo-status-bar';
import { useColorScheme } from 'react-native';
import { useEffect } from 'react';
import AppNavigator from './src/navigation/AppNavigator';
import ErrorBoundary from './src/components/ErrorBoundary';
import { ProvidersProvider } from './src/contexts/ProvidersContext';
import { startBackgroundService } from './modules/expo-rust-bridge';

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

  return (
    <ErrorBoundary>
      <ProvidersProvider>
        <AppNavigator />
        <StatusBar style={colorScheme === 'dark' ? 'light' : 'dark'} />
      </ProvidersProvider>
    </ErrorBoundary>
  );
}
