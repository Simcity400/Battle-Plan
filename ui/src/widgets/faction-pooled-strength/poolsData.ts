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
    pool === 'resources' ? data.resources != null
      : pool === 'manpower' ? data.manpowerUnlocked !== false && data.manpower != null
        : pool === 'research' ? data.researchUnlocked !== false && (data.researchBaseRaw != null || data.researchCountries != null)
          : data.doctrineFolders != null && data.doctrineFolders.length > 0);
}

/** The saved pool while the faction has it, else the first it has. */
export const shownPool = (offered: readonly Pool[], saved: unknown): Pool | null =>
  offered.includes(saved as Pool) ? saved as Pool : offered[0] ?? null;

export interface MemberAmount { key: string; tag: string | null; value: number }

/** Manpower contributions, each recorded occurrence kept (repeats are native, never merged), largest first. */
export function manpowerRows(data: FactionDetailReply): MemberAmount[] {
  return (data.manpower ?? []).map(entry => ({ key: String(entry.ordinal), tag: entry.tag, value: entry.value }))
    .sort((a, b) => b.value - a.value || Number(a.key) - Number(b.key));
}

/** A part of a pool's total in percent, one decimal; none without a positive total. */
export function shareOf(value: number, total: number | null): string {
  if (total == null || !(total > 0)) return '—';
  return `${(Math.round(value / total * 1000) / 10).toFixed(1)}%`;
}
