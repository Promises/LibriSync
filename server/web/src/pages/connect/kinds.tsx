import type { ComponentType } from 'react';
import { BellRing, Library, Webhook, type LucideProps } from 'lucide-react';
import type { NotificationKind } from '../../api/types';
import { ChoiceIcon, ChoiceList } from '../../components/ChoiceList';
import { Modal } from '../../components/Modal';

export const KIND_META: Record<NotificationKind, { label: string; description: string; Icon: ComponentType<LucideProps> }> = {
  audiobookshelf: {
    label: 'Audiobookshelf',
    description: 'Scan an Audiobookshelf library as soon as a book is downloaded.',
    Icon: Library,
  },
  apprise: {
    label: 'Apprise',
    description: 'Discord, Telegram, ntfy, Gotify, email and more, through an apprise-api server.',
    Icon: BellRing,
  },
  webhook: {
    label: 'Webhook',
    description: 'Send a Sonarr-style JSON payload to any URL.',
    Icon: Webhook,
  },
};

const ORDER: NotificationKind[] = ['audiobookshelf', 'apprise', 'webhook'];

export function KindPicker({ open, onClose, onPick }: { open: boolean; onClose: () => void; onPick: (k: NotificationKind) => void }) {
  return (
    <Modal open={open} onClose={onClose} title="Add connection" size="md">
      <ChoiceList
        onPick={onPick}
        choices={ORDER.map((k) => {
          const m = KIND_META[k];
          return {
            key: k,
            icon: (
              <ChoiceIcon>
                <m.Icon size={20} aria-hidden />
              </ChoiceIcon>
            ),
            title: m.label,
            description: m.description,
          };
        })}
      />
    </Modal>
  );
}
