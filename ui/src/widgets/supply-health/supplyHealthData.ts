import { countryKey } from '../../dashboard/countryKey.ts';
import { formatShare as formatDerivedShare } from '../../families/valueFormat.ts';
import type { SeriesState } from '../../generated/dashboard';

export const STATES = ['noSupply', 'strained', 'inSupply'] as const;
export type SupplyState = (typeof STATES)[number];
export const STATE_LABELS = { noSupply: 'No supply', strained: 'Strained', inSupply: 'In supply' } as const;
export const SHARE_MEASURES = ['noSupplyShare', 'strainedShare', 'inSupplyShare'] as const;
export const SUPPLY_FAMILY = 'supply-health';

export type SupplyCounts = Readonly<Record<SupplyState, number>>;
export type SupplyAt = { counts: SupplyCounts } | { missing: string } | undefined;

export const totalDivisions = (counts: SupplyCounts) => STATES.reduce((sum, state) => sum + counts[state], 0);
export const formatShare = (count: number, total: number) =>
  total ? formatDerivedShare(count / total) : 'Not known';

export function supplyAt(replies: readonly (readonly SeriesState[])[] | undefined, tag: string, hour: number): SupplyAt {
  if (!replies || replies.length !== STATES.length) return undefined;
  const found = replies.map(states => states.find(state => countryKey(state.country) === tag && state.hour === hour));
  if (found.some(state => !state)) return undefined;
  const missing = found.find(state => !state!.run);
  if (missing) return { missing: missing!.missing ?? 'Not recorded at this hour' };
  return { counts: Object.fromEntries(STATES.map((state, at) => [state, Number(found[at]!.run!.valueRaw)])) as SupplyCounts };
}
