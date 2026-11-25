import type { LegacyWidgetEntry } from '../families/manifest.ts';

export const LEGACY_WIDGETS: readonly LegacyWidgetEntry[] = [
  { kind: 'map', load: () => import('./map') },
  { kind: 'naval-battles', load: () => import('./naval-battles') },
];
