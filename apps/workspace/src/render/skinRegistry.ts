import type { WorkspaceSkin } from './skinTypes';

/** Metadata shown in the skin picker before the module is loaded. */
export interface SkinEntry {
  id: string;
  name: string;
  description: string;
  load: () => Promise<WorkspaceSkin>;
}

/**
 * Central registry of every available skin.
 *
 * Each entry carries only lightweight metadata; the actual component
 * tree is behind a dynamic import so unused skins never enter the
 * main bundle.
 */
export const skinRegistry: SkinEntry[] = [
  {
    id: 'ascii',
    name: 'ASCII Terminal',
    description: 'Green-on-black terminal aesthetic with box-drawing characters.',
    load: () => import('./skins/ascii/index').then((m) => m.default),
  },
  {
    id: 'emoji',
    name: 'Emoji Factory',
    description: 'Colourful emoji-driven skin with a lighter palette.',
    load: () => import('./skins/emoji/index').then((m) => m.default),
  },
];

/** Convenience: the default skin ID used when nothing is stored. */
export const DEFAULT_SKIN_ID = 'ascii';
