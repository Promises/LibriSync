import { useState } from 'react';
import { Plus, Users } from 'lucide-react';
import { useAccounts } from '../../api/queries';
import { Button } from '../../components/Button';
import { EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import { PageHeader } from '../../components/Navigation';
import { AccountCard } from './AccountCard';
import { AddAccountDialog } from './AddAccountDialog';
import styles from './AccountsPage.module.css';

export function AccountsPage() {
  const accounts = useAccounts();
  const [adding, setAdding] = useState(false);

  const addButton = (
    <Button variant="primary" icon={<Plus size={16} />} onClick={() => setAdding(true)}>
      Add account
    </Button>
  );

  return (
    <>
      <PageHeader
        title="Accounts"
        subtitle="Store accounts LibriSync syncs and downloads from."
        actions={accounts.data && accounts.data.length > 0 ? addButton : undefined}
      />
      {accounts.isPending ? (
        <SkeletonRows rows={3} label="Loading accounts" />
      ) : accounts.isError ? (
        <ErrorState error={accounts.error} onRetry={() => void accounts.refetch()} />
      ) : accounts.data.length === 0 ? (
        <EmptyState icon={<Users size={24} />} title="No accounts yet" action={addButton}>
          Add an Audible, Libro.fm or Chirp account to start syncing. LibriSync fetches your library straight away
          and downloads new purchases automatically.
        </EmptyState>
      ) : (
        <div className={styles.grid}>
          {accounts.data.map((a) => (
            <AccountCard key={a.id} account={a} />
          ))}
        </div>
      )}
      <AddAccountDialog open={adding} onClose={() => setAdding(false)} />
    </>
  );
}
