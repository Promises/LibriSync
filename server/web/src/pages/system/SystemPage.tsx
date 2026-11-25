import { Navigate, useParams } from 'react-router';
import { PageHeader, TabNav } from '../../components/Navigation';
import { LogsTab } from './LogsTab';
import { StatusTab } from './StatusTab';
import { TasksTab } from './TasksTab';

const TABS = ['status', 'tasks', 'logs'] as const;
type Tab = (typeof TABS)[number];

export function SystemPage() {
  const { tab } = useParams();
  if (!TABS.includes(tab as Tab)) return <Navigate to="/system/status" replace />;
  return (
    <>
      <PageHeader title="System" />
      <TabNav
        label="System"
        tabs={[
          { to: '/system/status', label: 'Status' },
          { to: '/system/tasks', label: 'Tasks' },
          { to: '/system/logs', label: 'Logs' },
        ]}
      />
      {tab === 'status' && <StatusTab />}
      {tab === 'tasks' && <TasksTab />}
      {tab === 'logs' && <LogsTab />}
    </>
  );
}
