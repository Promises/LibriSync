import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Play } from 'lucide-react';
import { systemApi } from '../../api/endpoints';
import { keys, useTasks } from '../../api/queries';
import type { ScheduledTask, TaskName } from '../../api/types';
import { Badge } from '../../components/Badge';
import { Button } from '../../components/Button';
import { EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import t from '../../components/Table.module.css';
import { toast } from '../../components/Toast';
import { useNow } from '../../hooks/useNow';
import { formatDateTime, formatRelative } from '../../lib/format';

function formatInterval(minutes: number): string {
  if (minutes <= 0) return 'On demand';
  if (minutes % 1440 === 0) return minutes === 1440 ? 'Daily' : `Every ${minutes / 1440} days`;
  if (minutes % 60 === 0) return minutes === 60 ? 'Hourly' : `Every ${minutes / 60} hours`;
  return `Every ${minutes} min`;
}

export function TasksTab() {
  const tasks = useTasks();
  const client = useQueryClient();
  const now = useNow();
  const run = useMutation({
    mutationFn: (name: TaskName) => systemApi.runTask(name),
    meta: { errorTitle: 'Could not start the task' },
    onSuccess: (_r, name) => {
      client.setQueryData<ScheduledTask[]>(keys.tasks, (l) => l?.map((x) => (x.name === name ? { ...x, running: true } : x)));
      toast.info('Task started');
    },
  });

  if (tasks.isPending) return <SkeletonRows rows={4} label="Loading tasks" />;
  if (tasks.isError) return <ErrorState error={tasks.error} onRetry={() => void tasks.refetch()} />;
  if (tasks.data.length === 0) return <EmptyState title="No scheduled tasks" />;

  return (
    <div className={t.wrap}>
      <table className={t.table}>
        <thead>
          <tr>
            <th>Task</th>
            <th className={t.hideOnPhone}>Interval</th>
            <th>Last run</th>
            <th className={t.hideOnPhone}>Next run</th>
            <th>
              <span className="visually-hidden">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {tasks.data.map((task) => (
            <tr key={task.name}>
              <td>
                <strong>{task.displayName}</strong>
                {task.running && (
                  <>
                    {' '}
                    <Badge tone="accent">Running</Badge>
                  </>
                )}
              </td>
              <td className={`${t.hideOnPhone} ${t.nowrap}`}>{formatInterval(task.intervalMinutes)}</td>
              <td className={t.nowrap} title={formatDateTime(task.lastRun)}>
                {formatRelative(task.lastRun, now)}
              </td>
              <td className={`${t.hideOnPhone} ${t.nowrap}`} title={formatDateTime(task.nextRun)}>
                {task.nextRun ? formatRelative(task.nextRun, now) : '—'}
              </td>
              <td className={t.nowrap}>
                <Button
                  size="sm"
                  icon={<Play size={14} />}
                  onClick={() => run.mutate(task.name)}
                  disabled={task.running}
                  loading={run.isPending && run.variables === task.name}
                >
                  Run now
                </Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
