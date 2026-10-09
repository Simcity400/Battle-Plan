import type { FactionTotalsParams } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import { factionTotalsRequest } from '../../dashboard/factionTotals.ts';

/** Shapes glide within recorded stretches; lookup labels remain exact. */
export const SMOOTH_TOTALS_PRESENTATION = { exact: false, held: false, lineColourHours: 'labels', raceTooltip: 'hover',
  raceEmptyState: false } as const;

/** An all-factions choice asks for the recorded catalogue's size, including
 * factions that only exist partway through. No chosen ceiling reaches the wire. */
export function factionRaceRequest(branchId: string, selection: CountrySelection,
  source: FactionTotalsParams['source'], recordedFactions: number, max: number): FactionTotalsParams | null {
  return factionTotalsRequest(branchId, selection, source, Math.min(max, Math.max(1, recordedFactions)));
}
