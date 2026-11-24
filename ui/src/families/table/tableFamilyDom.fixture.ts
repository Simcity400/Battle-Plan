import type { ScoreboardColumn } from '../../generated/dashboard';
export { scoreboardColumnsSpec, scoreboardProjection } from '../../widgets/scoreboard/scoreboardProjection.ts';

export const SCOREBOARD_FIXTURE_COLUMNS: ScoreboardColumn[] = [
  { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null, family: { kind: 'f', measure: 'm' }, partial: null, noneAtZero: false },
  { key: 'casualties', label: 'Casualties', unit: 'people', decimals: 0, available: true, reason: null, family: { kind: 'c', measure: 't' }, partial: null, noneAtZero: true },
];
