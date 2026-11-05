import { useEffect, useState } from 'react';
import { getStageProgress } from '../../modules/expo-rust-bridge';

const POLL_MS = 3000;

/**
 * How many books the download service is working on or holding in its queue — the
 * Downloads tab badge. Reads the service's in-memory stage store, not the database, so
 * polling it in the background is cheap.
 */
export function useActiveDownloadCount(): number {
  const [count, setCount] = useState(0);

  useEffect(() => {
    const read = () => {
      try {
        setCount(Object.keys(getStageProgress()).length);
      } catch {
        // No native module (e.g. a JS-only dev session): show no badge.
        setCount(0);
      }
    };
    read();
    const timer = setInterval(read, POLL_MS);
    return () => clearInterval(timer);
  }, []);

  return count;
}
