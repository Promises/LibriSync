import React from 'react';
import Button from './Button';

interface Props {
  /** How many real accounts this provider has — gates the Sync All button. */
  accountCount: number;
  isSyncing: boolean;
  /** True once a sync has run, so the button reads "Sync Again". */
  hasSynced: boolean;
  onSync: () => void;
  onSyncAll: () => void;
  onSignOut: () => void;
  signOutTitle?: string;
}

/**
 * Sync / Sync All / Sign Out for one provider's account screen.
 *
 * "Sync All Accounts" only appears with more than one account — with a single
 * account it does exactly what "Sync Library" does. Note `accountCount` must be
 * scoped to this provider: counting every provider's accounts is what made the
 * button show up on the Audible screen for users with one Audible account.
 */
export default function AccountActions({
  accountCount,
  isSyncing,
  hasSynced,
  onSync,
  onSyncAll,
  onSignOut,
  signOutTitle = 'Log Out',
}: Props) {
  return (
    <>
      <Button
        title={isSyncing ? 'Syncing...' : hasSynced ? 'Sync Again' : 'Sync Library'}
        onPress={onSync}
        variant="filled"
        state="warning"
        disabled={isSyncing}
        style={{ marginTop: 8 }}
      />

      {accountCount > 1 && (
        <Button
          title={isSyncing ? 'Syncing...' : 'Sync All Accounts'}
          onPress={onSyncAll}
          variant="outlined"
          state="warning"
          disabled={isSyncing}
          style={{ marginTop: 8 }}
        />
      )}

      <Button
        title={signOutTitle}
        onPress={onSignOut}
        variant="outlined"
        state="error"
        style={{ marginTop: 8 }}
      />
    </>
  );
}
