import type { LegacyWidgetEntry } from '../families/manifest.ts';

export const LEGACY_WIDGETS: readonly LegacyWidgetEntry[] = [
  { kind: 'naval-battles', load: () => import('./naval-battles') },
];
