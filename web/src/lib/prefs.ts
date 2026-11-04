// Per-viewer conveniences remembered in this browser only: table density, hidden columns, whether
// the positions panel or the assumptions rail is open. Every read and write is guarded, so a
// blocked or cleared store simply means the default.
import { useCallback, useState } from 'react';

export function readPref<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(`nbb-${key}`);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

export function writePref<T>(key: string, value: T) {
  try {
    localStorage.setItem(`nbb-${key}`, JSON.stringify(value));
  } catch {
    // storage unavailable: the preference lives for this page only
  }
}

export function usePref<T>(key: string, fallback: T): [T, (next: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => readPref(key, fallback));
  const set = useCallback(
    (next: T | ((prev: T) => T)) => {
      setValue((prev) => {
        const v = typeof next === 'function' ? (next as (p: T) => T)(prev) : next;
        writePref(key, v);
        return v;
      });
    },
    [key],
  );
  return [value, set];
}
