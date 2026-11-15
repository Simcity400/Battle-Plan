import { usNumber } from '../../charts/numberFormat.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { SortValue } from '../../charts/table/logic';
import { manpowerKey, manpowerTotalKey } from '../../dashboard/factionDetails.ts';
import type { FactionDetailReply } from '../../generated/dashboard';

export type Pool = 'resources' | 'manpower' | 'research' | 'doctrine';
export const POOL_CHOICES: readonly { value: Pool; label: string }[] = [
  { value: 'resources', label: 'Resources' }, { value: 'manpower', label: 'Manpower' },
  { value: 'research', label: 'Research' }, { value: 'doctrine', label: 'Doctrine' },
];

/** The pools a faction has at the hour: only what its recorded state supplies. A locked
 * section is not offered; an unavailable read is not "nothing pooled". */
export function poolsOffered(data: FactionDetailReply): Pool[] {
  return POOL_CHOICES.map(choice => choice.value).filter(pool =>
    pool === 'resources' ? data.resources !== null
      : pool === 'manpower' ? data.manpowerUnlocked !== false && data.manpower !== null
        : pool === 'research' ? data.researchUnlocked !== false && data.researchCountries !== null
          : data.doctrineFolders !== null && data.doctrineFolders.length > 0);
}

/** The shown pool's total or count for the header; a total not recorded says so, never zero. */
export function poolHeading(data: FactionDetailReply, pool: Pool, values: LiveValues<number>): string {
  if (pool === 'manpower') {
    const total = values.get(manpowerTotalKey);
    return total === undefined ? 'Total not recorded' : `${usNumber.format(total)} manpower`;
  }
  return pool === 'resources' ? `${data.resources?.length ?? 0} resources`
    : pool === 'research' ? `${data.researchCountries?.length ?? 0} sharing`
      : `${data.doctrineFolders?.length ?? 0} doctrines`;
}

/** The saved pool while the faction has it, else the first it has. */
export const shownPool = (offered: readonly Pool[], saved: unknown): Pool | null =>
  offered.includes(saved as Pool) ? saved as Pool : offered[0] ?? null;

/** A manpower contribution's row: its live cells' row (`manpower:<ordinal>`) and its country at the hour. */
export interface MemberAmount { key: string; ordinal: number; tag: string | null }

/** Manpower contributions, each recorded occurrence kept (repeats are native, never merged), in native order; the
 * table sorts them by their live values. */
export function manpowerRows(data: FactionDetailReply): MemberAmount[] {
  return (data.manpower ?? []).map(entry => ({ key: `manpower:${entry.ordinal}`, ordinal: entry.ordinal, tag: entry.tag }));
}

/** A part of a pool's total in percent, one decimal; none without a positive total. */
export function shareOf(value: number | undefined, total: number | undefined): string {
  if (value === undefined || total === undefined || !(total > 0)) return '—';
  return `${(Math.round(value / total * 1000) / 10).toFixed(1)}%`;
}

/** The manpower table's live source: `manpower` a contribution, `share` it over the pool's total. */
export function manpowerTableValues(values: LiveValues<number>): LiveValues<SortValue> {
  const split = (key: string) => { const at = key.lastIndexOf('\u0000'); return [key.slice(0, at), key.slice(at + 1)] as const; };
  const value = (row: string) => values.get(liveKey(row, 'value'));
  return {
    get: key => {
      const [row, column] = split(key);
      if (column === 'manpower') return value(row);
      if (column !== 'share') return undefined;
      const [amount, total] = [value(row), values.get(manpowerTotalKey)];
      return amount === undefined || total === undefined || !(total > 0) ? undefined : amount / total;
    },
    missing: key => {
      const [row, column] = split(key);
      return column === 'share' && values.get(manpowerTotalKey) === undefined ? values.missing(manpowerTotalKey) || 'Total not recorded'
        : values.missing(liveKey(row, 'value')) || 'Not recorded';
    },
  };
}

/** A contribution's amount for its tooltip. */
export const memberAmount = (values: LiveValues<number>, ordinal: number) => values.get(manpowerKey(ordinal));
