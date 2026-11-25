import type { LegacyWidgetEntry } from '../families/manifest.ts';

export const LEGACY_WIDGETS: readonly LegacyWidgetEntry[] = [
  { kind: 'construction-log', load: () => import('./construction-log') },
  { kind: 'division-log', load: () => import('./division-log') },
  { kind: 'division-service-record', load: () => import('./division-service-record') },
  { kind: 'espionage-log', load: () => import('./espionage-log') },
  { kind: 'fuel-branches', view: 'time', label: 'Over time', load: () => import('./fuel-branches') },
  { kind: 'land-battles', load: () => import('./land-battles') },
  { kind: 'map', load: () => import('./map') },
  { kind: 'naval-battles', load: () => import('./naval-battles') },
  { kind: 'peace-deals', load: () => import('./peace-deals') },
  { kind: 'ships-sunk', load: () => import('./ships-sunk') },
];
