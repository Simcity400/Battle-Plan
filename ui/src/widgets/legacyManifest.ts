import type { LegacyWidgetEntry } from '../families/manifest.ts';

export const LEGACY_WIDGETS: readonly LegacyWidgetEntry[] = [
  { kind: 'land-battles', load: () => import('./land-battles') },
  { kind: 'map', load: () => import('./map') },
  { kind: 'naval-battles', load: () => import('./naval-battles') },
  { kind: 'peace-deals', load: () => import('./peace-deals') },
  { kind: 'ships-sunk', load: () => import('./ships-sunk') },
];
