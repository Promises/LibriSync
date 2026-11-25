import { useCallback, useState } from 'react';

// localStorage can throw (private mode, blocked site data) or be empty; every access is
// guarded and the app works the same without it, just without remembering.

export function readStorage(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writeStorage(key: string, value: string | null): void {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch {
    // Not persisted; the in-memory value still applies for this session.
  }
}

/** A string-union preference remembered per browser. */
export function useStoredChoice<T extends string>(
  key: string,
  allowed: readonly T[],
  fallback: T,
): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => {
    const stored = readStorage(key);
    return stored !== null && (allowed as readonly string[]).includes(stored) ? (stored as T) : fallback;
  });
  const set = useCallback(
    (next: T) => {
      setValue(next);
      writeStorage(key, next);
    },
    [key],
  );
  return [value, set];
}
