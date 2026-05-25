import { useState, useEffect, useCallback } from 'react';
import type { WorkspaceSkin } from './skinTypes';
import { skinRegistry, DEFAULT_SKIN_ID } from './skinRegistry';
import type { SkinEntry } from './skinRegistry';

const STORAGE_KEY = 'workspace-skin';

function readStoredSkinId(): string {
  try {
    return localStorage.getItem(STORAGE_KEY) ?? DEFAULT_SKIN_ID;
  } catch {
    return DEFAULT_SKIN_ID;
  }
}

function writeStoredSkinId(id: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, id);
  } catch {
    // Silently ignore (e.g. private browsing quota).
  }
}

export interface UseSkinResult {
  /** The fully-loaded skin, or null while loading. */
  skin: WorkspaceSkin | null;
  /** Whether the skin module is currently being fetched. */
  loading: boolean;
  /** Switch to a different skin by ID. */
  setSkin: (id: string) => void;
  /** All registered skins (metadata only -- not yet loaded). */
  availableSkins: SkinEntry[];
}

export function useSkin(): UseSkinResult {
  const [activeSkinId, setActiveSkinId] = useState<string>(readStoredSkinId);
  const [skin, setSkinState] = useState<WorkspaceSkin | null>(null);
  const [loading, setLoading] = useState(true);

  // Load the active skin module whenever the id changes.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    // Set the data-skin attribute so CSS custom property overrides activate.
    document.documentElement.setAttribute('data-skin', activeSkinId);

    const entry = skinRegistry.find((s) => s.id === activeSkinId) ?? skinRegistry[0];
    if (!entry) return;

    entry.load().then((loaded) => {
      if (cancelled) return;
      setSkinState(loaded);
      setLoading(false);
    }).catch((err) => {
      console.error(`[skin] Failed to load skin "${activeSkinId}":`, err);
      // Fallback: try the default skin if it differs.
      if (activeSkinId !== DEFAULT_SKIN_ID) {
        const fallback = skinRegistry.find((s) => s.id === DEFAULT_SKIN_ID);
        fallback?.load().then((loaded) => {
          if (cancelled) return;
          setSkinState(loaded);
          setLoading(false);
        });
      }
    });

    return () => { cancelled = true; };
  }, [activeSkinId]);

  const setSkin = useCallback((id: string) => {
    writeStoredSkinId(id);
    setActiveSkinId(id);
  }, []);

  return {
    skin,
    loading,
    setSkin,
    availableSkins: skinRegistry,
  };
}
