import { useEffect, useState } from 'react';
import { MutationCache, QueryClient, QueryClientProvider, useQueryClient } from '@tanstack/react-query';
import { BrowserRouter, Navigate, Route, Routes } from 'react-router';
import { ApiError, errorMessage, onUnauthorized } from './api/client';
import { keys, useAuthState } from './api/queries';
import type { AuthState } from './api/types';
import { ConfirmProvider } from './components/Confirm';
import { ErrorState } from './components/Feedback';
import { Spinner } from './components/Spinner';
import { toast, ToastViewport } from './components/Toast';
import { Layout } from './layout/Layout';
import { AccountsPage } from './pages/accounts/AccountsPage';
import { ActivityPage } from './pages/activity/ActivityPage';
import { AuthPage } from './pages/auth/AuthPage';
import { ConnectPage } from './pages/connect/ConnectPage';
import { LibraryPage } from './pages/library/LibraryPage';
import { NotFoundPage } from './pages/NotFoundPage';
import { SettingsPage } from './pages/settings/SettingsPage';
import { SystemPage } from './pages/system/SystemPage';
import { WantedPage } from './pages/wanted/WantedPage';
import styles from './App.module.css';

declare module '@tanstack/react-query' {
  interface Register {
    mutationMeta: {
      /** Toast title on failure; omit to use a generic one. */
      errorTitle?: string;
      /** The caller shows the error itself (inline form errors); no toast. */
      silent?: boolean;
    };
  }
}

function makeQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 10_000,
        // A 4xx will not fix itself by asking again.
        retry: (count, error) =>
          !(error instanceof ApiError && error.status >= 400 && error.status < 500) && count < 2,
      },
    },
    mutationCache: new MutationCache({
      onError: (error, _vars, _ctx, mutation) => {
        if (mutation.meta?.silent) return;
        if (error instanceof ApiError && error.status === 401) return;
        toast.error(mutation.meta?.errorTitle ?? 'Something went wrong', errorMessage(error));
      },
    }),
  });
}

/** Sends the user to the login page whenever any API call reports a lost session. */
function UnauthorizedWatcher() {
  const client = useQueryClient();
  useEffect(
    () =>
      onUnauthorized(() => {
        client.setQueryData<AuthState>(keys.authState, (s) =>
          s ? { ...s, authenticated: false } : s,
        );
        void client.invalidateQueries({ queryKey: keys.authState });
      }),
    [client],
  );
  return null;
}

function AuthGate() {
  const auth = useAuthState();

  if (auth.isPending) {
    return (
      <div className={styles.center}>
        <Spinner size={28} label="Loading LibriSync" />
      </div>
    );
  }
  if (auth.isError) {
    return (
      <div className={styles.center}>
        <div className={styles.narrow}>
          <ErrorState error={auth.error} onRetry={() => void auth.refetch()} />
        </div>
      </div>
    );
  }
  if (auth.data.setupRequired) return <AuthPage mode="setup" />;
  if (!auth.data.authenticated) return <AuthPage mode="login" />;

  return (
    <Routes>
      <Route element={<Layout username={auth.data.username} />}>
        <Route index element={<Navigate to="/library" replace />} />
        <Route path="library" element={<LibraryPage />} />
        <Route path="activity" element={<Navigate to="/activity/queue" replace />} />
        <Route path="activity/:tab" element={<ActivityPage />} />
        <Route path="wanted" element={<WantedPage />} />
        <Route path="accounts" element={<AccountsPage />} />
        <Route path="settings" element={<SettingsPage />} />
        <Route path="connect" element={<ConnectPage />} />
        <Route path="system" element={<Navigate to="/system/status" replace />} />
        <Route path="system/:tab" element={<SystemPage />} />
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  );
}

export function App() {
  const [client] = useState(makeQueryClient);
  return (
    <QueryClientProvider client={client}>
      <BrowserRouter>
        <ConfirmProvider>
          <UnauthorizedWatcher />
          <AuthGate />
          <ToastViewport />
        </ConfirmProvider>
      </BrowserRouter>
    </QueryClientProvider>
  );
}
