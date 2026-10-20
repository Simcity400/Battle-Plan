import type { CountryGroupReply, LendLeaseBalanceReply, LendLeaseHistory } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import { groupMembers, groupRuns, groupUnobserved } from '../../dashboard/groups.ts';
import { object } from '../../dashboard/validation.ts';
import { bigAt, fixedText, isEconomyRuns, isHolderTags, runAt, tagAt, validateEconomyReply } from '../../dashboard/economyHistory.ts';

export const LEND_LEASE_BALANCE = 'lend-lease-balance';
export type BalanceMeasure = 'industry' | 'fuel';

export function validateLendLeaseBalance(value: unknown): LendLeaseBalanceReply {
  return validateEconomyReply<LendLeaseBalanceReply>(value, reply => Number.isSafeInteger(reply.scale) && (reply.scale as number) > 0 && isEconomyRuns(reply.known)
    && (reply.holder === null || typeof reply.holder === 'string')
    && Array.isArray(reply.histories) && reply.histories.every(history => object(history) && typeof history.holder === 'string'
      && isHolderTags(history.tags) && Array.isArray(history.values) && history.values.length === 4 && history.values.every(runs => isEconomyRuns(runs))),
  'lend-lease balance');
}

/** One country's (or partner's) cumulative totals at an hour, in one measure. */
export interface BalanceRow {
  holder: string;
  tag: string;
  sent: bigint | null;
  received: bigint | null;
}

/** The game's recorded histories of `measure` at `hour`: sent then received. */
export function balanceAt(history: LendLeaseHistory, measure: BalanceMeasure, hour: number): BalanceRow | null {
  const tag = tagAt(history.tags, hour);
  if (tag === null) return null;
  const offset = measure === 'industry' ? 0 : 1;
  const sent = bigAt(history.values[offset], hour), received = bigAt(history.values[2 + offset], hour);
  if (sent === null && received === null) return null;
  return { holder: history.holder, tag, sent, received };
}

const total = (row: BalanceRow) => (row.sent ?? 0n) + (row.received ?? 0n);

/** The countries a selection names at `hour`: chosen tags (null for every
 * country) and a top group's count; null while the groups load. */
export function balanceSelection(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, lastHour: number): { tags: Set<string> | null; limit: number; membersUnknown?: true } | null {
  if (selection.kind === 'tags') return { tags: new Set(selection.tags), limit: Infinity };
  if (selection.kind === 'selected') return { tags: new Set(fallback), limit: Infinity };
  if (selection.kind !== 'group') return { tags: null, limit: Infinity };
  if (selection.group.type === 'top') return { tags: null, limit: selection.group.count };
  if (!groups) return null;
  const runs = groupRuns(groups, selection.group);
  const members = runs && groupMembers(runs, selection.membership, hour, lastHour, Infinity, groupUnobserved(groups, selection.group));
  // Membership not observed at the hour: no country is shown, and none is known.
  return members ? { tags: new Set(members), limit: Infinity } : { tags: new Set(), limit: Infinity, membersUnknown: true };
}

/** Rows at `hour` ranked by sent plus received, ties by holder: those the selection
 * keeps, at most a top group's count. A country with neither leaves the chart. */
export function rankedAt(reply: LendLeaseBalanceReply, measure: BalanceMeasure, hour: number, selection: CountrySelection,
  fallback: readonly string[], groups: CountryGroupReply | undefined, lastHour: number): BalanceRow[] | null {
  const chosen = balanceSelection(selection, fallback, groups, hour, lastHour);
  if (!chosen) return null;
  const { tags, limit } = chosen;
  const rows = reply.histories.map(history => balanceAt(history, measure, hour))
    .filter((row): row is BalanceRow => row !== null && (row.sent ?? 0n) !== 0n || row !== null && (row.received ?? 0n) !== 0n)
    .filter(row => !tags || tags.has(row.tag));
  rows.sort((a, b) => { const x = total(a), y = total(b); return x === y ? Number(a.holder) - Number(b.holder) : x > y ? -1 : 1; });
  return rows.slice(0, limit);
}

export type EmptyBalance = 'unrecorded' | 'members-unrecorded' | 'zero' | 'partial' | 'none-recorded';

/** Why no bar shows at `hour`, from the selected countries' own histories: not
 * read then ('unrecorded'); every selected country's recorded totals are exactly
 * zero ('zero', proven); some are zero and the rest have no recorded total
 * ('partial'); or none has one (outside the war rosters the game keeps them for,
 * 'none-recorded'). A group whose membership was not observed then is
 * 'members-unrecorded'. Every country, or a top group, proves zero from every
 * history recorded then. */
export function emptyBalance(reply: LendLeaseBalanceReply, measure: BalanceMeasure, hour: number, selection: CountrySelection,
  fallback: readonly string[], groups: CountryGroupReply | undefined, lastHour: number): EmptyBalance {
  if (!runAt(reply.known, hour)) return 'unrecorded';
  const chosen = balanceSelection(selection, fallback, groups, hour, lastHour);
  if (!chosen) return 'none-recorded';
  if (chosen.membersUnknown) return 'members-unrecorded';
  const rows = new Map<string, BalanceRow>();
  for (const history of reply.histories) {
    const row = balanceAt(history, measure, hour);
    if (row && (!chosen.tags || chosen.tags.has(row.tag))) rows.set(row.tag, row);
  }
  const zero = (row: BalanceRow | undefined) => !!row && row.sent === 0n && row.received === 0n;
  const proven = [...rows.values()].filter(zero).length;
  const wanted = chosen.tags ? chosen.tags.size : rows.size;
  return proven === 0 ? 'none-recorded' : proven === wanted ? 'zero' : 'partial';
}

/** Bars draw positive amounts only; the written values stay exact and signed. */
export const barValue = (value: bigint | null) => value === null ? null : Number(value > 0n ? value : 0n) / 100000;
export const amountText = (value: bigint | null, scale: number) => value === null ? '—' : fixedText(value, BigInt(scale));

/** The first hour any measure of the history was recorded. */
export const firstRecorded = (history: LendLeaseHistory): number | null => {
  const starts = history.values.flatMap(runs => runs.length ? [runs[0][0]] : []);
  return starts.length ? Math.min(...starts) : null;
};
