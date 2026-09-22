'use client';

import { useCallback, useEffect, useState } from 'react';

export interface Macro {
  id: string;
  name: string;
  script: string;
  sendEnter?: boolean; // Whether to send Enter after each line (default: true)
  isBuiltIn?: boolean; // Built-in macros cannot be edited/deleted
}

export const MACROS_STORAGE_KEY = 'par-term-macros';

/** True when `value` is a well-formed `Macro`: has string `name`/`script`. */
function isValidMacro(value: unknown): value is Macro {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.id === 'string' &&
    typeof candidate.name === 'string' &&
    typeof candidate.script === 'string'
  );
}

/** Parse and validate the macro list persisted at `MACROS_STORAGE_KEY`. */
export function parseStoredMacros(raw: string): Macro[] {
  const parsed: unknown = JSON.parse(raw);
  if (!Array.isArray(parsed)) return [];
  return parsed.filter(isValidMacro);
}

/**
 * Loads and persists the user's onscreen-keyboard macros in `localStorage`
 * (QA-118). A corrupted or unexpectedly-shaped stored value (not an array,
 * or an array containing entries missing `name`/`script`) yields `[]`
 * instead of throwing or setting invalid state that later macro-playback
 * code would crash on.
 */
export function useStoredMacros() {
  const [macros, setMacros] = useState<Macro[]>([]);

  // Load macros from localStorage after mount. See app/page.tsx for the
  // rationale on preferring post-mount hydration over lazy initializers in
  // a Next.js SSR context.
  useEffect(() => {
    try {
      const stored = localStorage.getItem(MACROS_STORAGE_KEY);
      if (stored) {
        setMacros(parseStoredMacros(stored));
      }
    } catch (e) {
      console.error('Failed to load macros:', e);
    }
  }, []);

  const saveMacros = useCallback((newMacros: Macro[]) => {
    setMacros(newMacros);
    try {
      localStorage.setItem(MACROS_STORAGE_KEY, JSON.stringify(newMacros));
    } catch (e) {
      console.error('Failed to save macros:', e);
    }
  }, []);

  return { macros, saveMacros };
}
