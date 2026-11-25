import { Navigate, useParams } from 'react-router';
import { useQueue } from '../../api/queries';
import { PageHeader, TabNav } from '../../components/Navigation';
import { HistoryTab } from './HistoryTab';
import { QueueTab } from './QueueTab';

export function ActivityPage() {
  const { tab } = useParams();
  const queue = useQueue();
  if (tab !== 'queue' && tab !== 'history') return <Navigate to="/activity/queue" replace />;
  const count = queue.data?.length;
  return (
    <>
      <PageHeader title="Activity" subtitle="Downloads in progress and everything that has happened." />
      <TabNav
        label="Activity"
        tabs={[
          { to: '/activity/queue', label: count ? `Queue (${count})` : 'Queue' },
          { to: '/activity/history', label: 'History' },
        ]}
      />
      {tab === 'queue' ? <QueueTab /> : <HistoryTab />}
    </>
  );
}
