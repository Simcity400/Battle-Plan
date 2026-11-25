import type { LegacyWidgetEntry } from '../families/manifest.ts';

export const LEGACY_WIDGETS: readonly LegacyWidgetEntry[] = [
  { kind: 'autonomy', view: 'lines', label: 'Lines', load: () => import('./autonomy') },
  { kind: 'civil-wars', load: () => import('./civil-wars') },
  { kind: 'construction-log', load: () => import('./construction-log') },
  { kind: 'division-log', load: () => import('./division-log') },
  { kind: 'division-service-record', load: () => import('./division-service-record') },
  { kind: 'equipment-produced', view: 'lines', label: 'Lines', slots: ['render', 'countryChip'], load: () => import('./equipment-produced') },
  { kind: 'espionage-log', load: () => import('./espionage-log') },
  { kind: 'faction-pooled-strength', load: () => import('./faction-pooled-strength') },
  { kind: 'fuel-branches', load: () => import('./fuel-branches') },
  { kind: 'general-career', view: 'timeline', label: 'Timeline', load: () => import('./general-career') },
  { kind: 'land-battles', load: () => import('./land-battles') },
  { kind: 'map', load: () => import('./map') },
  { kind: 'naval-battles', load: () => import('./naval-battles') },
  { kind: 'naval-losses', view: 'lines', label: 'Lines', load: () => import('./naval-losses') },
  { kind: 'occupation-trend', view: 'mix', label: 'Mix', load: () => import('./occupation-trend') },
  { kind: 'peace-deals', load: () => import('./peace-deals') },
  { kind: 'ships-sunk', load: () => import('./ships-sunk') },
  { kind: 'territory', view: 'lines', label: 'Lines', load: () => import('./territory') },
];
