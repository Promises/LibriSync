import { Link } from 'react-router';
import { MapPinOff } from 'lucide-react';
import { EmptyState } from '../components/Feedback';

export function NotFoundPage() {
  return (
    <EmptyState icon={<MapPinOff size={24} />} title="Page not found" action={<Link to="/library">Go to the library</Link>}>
      There is nothing at this address.
    </EmptyState>
  );
}
