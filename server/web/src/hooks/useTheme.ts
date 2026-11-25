import { useCallback, useEffect, useState } from 'react';
import { readStorage, writeStorage } from './useLocalStorage';

export type ThemePreference = 'system' | 'light' | 'dark';

const KEY = 'librisync.theme';
const ORDER: ThemePreference[] = ['system', 'light', 'dark'];

function readPreference(): ThemePreference {
  const v = readStorage(KEY);
  return v === 'light' || v === 'dark' ? v : 'system';
}

/**
 * The manual theme toggle. `system` follows `prefers-color-scheme`; the others pin it by
 * setting `data-theme` on <html> (index.html applies the saved value before first paint).
 */
export function useTheme() {
  const [preference, setPreference] = useState<ThemePreference>(readPreference);

  useEffect(() => {
    const root = document.documentElement;
    if (preference === 'system') delete root.dataset.theme;
    else root.dataset.theme = preference;
    writeStorage(KEY, preference === 'system' ? null : preference);
  }, [preference]);

  const cycle = useCallback(() => {
    setPreference((p) => ORDER[(ORDER.indexOf(p) + 1) % ORDER.length] ?? 'system');
  }, []);

  return { preference, setPreference, cycle };
}
