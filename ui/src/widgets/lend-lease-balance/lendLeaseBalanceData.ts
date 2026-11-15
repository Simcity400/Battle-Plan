import type { CountryGroupReply, LendLeaseBalanceReply, LendLeasePartner, LendLeasePartnersReply } from '../../generated/dashboard';
import type { Run } from '../../charts/series';
import type { CountrySelection } from '../../dashboard/model';
import { groupMembers, groupRuns, groupUnobserved } from '../../dashboard/groups.ts';
import { formatAmount, rescale } from '../../charts/valueAxis.ts';

export const LEND_LEASE_BALANCE = 'lend-lease-balance';
export type BalanceMeasure = 'industry' | 'fuel';
const PLACES = 5;

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const raw = (value: unknown) => typeof value === 'string' && /^-?\d+$/.test(value);

export function validateLendLeaseBalance(value: unknown): LendLeaseBalanceReply {
  if (!object(value) || value.scale !== 100000 || !(value.holder === null || typeof value.holder === 'string')
    || !(value.missing === null || typeof value.missing === 'string') || !Array.isArray(value.warnings)
    || !Array.isArray(value.rows) || !value.rows.every((row, at, rows) => object(row) && Number.isSafeInteger(row.slot)
      && (at === 0 || (rows[at - 1] as { slot: number }).slot < (row.slot as number)) && (row.tag === null || typeof row.tag === 'string')
      && Array.isArray(row.values) && row.values.length === 4 && row.values.every(raw)))
    throw new Error('The lend-lease balance reply is malformed.');
  return value as unknown as LendLeaseBalanceReply;
}

/** One country's (or partner's) cumulative totals at the reply's hour, in one measure. */
export interface BalanceRow { slot: number; tag: string; sent: bigint; received: bigint }

/** Each row in `measure` with a tag: sent then received. */
export function balanceRows(reply: LendLeaseBalanceReply, measure: BalanceMeasure): BalanceRow[] {
  const offset = measure === 'industry' ? 0 : 1;
  return reply.rows.flatMap(row => row.tag === null ? [] : [{ slot: row.slot, tag: row.tag, sent: BigInt(row.values[offset]), received: BigInt(row.values[2 + offset]) }]);
}

/** The countries a selection names at `hour`: chosen tags (null for every country) and a top group's count; null while
 * the groups load. */
export function balanceSelection(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, lastHour: number): { tags: Set<string> | null; limit: number; membersUnknown?: true } | null {
  if (selection.kind === 'tags') return { tags: new Set(selection.tags), limit: Infinity };
  if (selection.kind === 'selected') return { tags: new Set(fallback), limit: Infinity };
  if (selection.kind !== 'group') return { tags: null, limit: Infinity };
  if (selection.group.type === 'top') return { tags: null, limit: selection.group.count };
  if (!groups) return null;
  const runs = groupRuns(groups, selection.group);
  const members = runs && groupMembers(runs, selection.membership, hour, lastHour, Infinity, groupUnobserved(groups, selection.group));
  return members ? { tags: new Set(members), limit: Infinity } : { tags: new Set(), limit: Infinity, membersUnknown: true };
}

const total = (row: BalanceRow) => row.sent + row.received;

/** Rows ranked by sent plus received, ties by slot: those the selection keeps, at most a top group's count. A country
 * with neither leaves the chart. */
export function rankedRows(rows: readonly BalanceRow[], chosen: { tags: Set<string> | null; limit: number }): BalanceRow[] {
  return rows.filter(row => (row.sent !== 0n || row.received !== 0n) && (!chosen.tags || chosen.tags.has(row.tag)))
    .sort((a, b) => { const x = total(a), y = total(b); return x === y ? a.slot - b.slot : x > y ? -1 : 1; })
    .slice(0, chosen.limit);
}

export type EmptyBalance = 'members-unrecorded' | 'zero' | 'partial' | 'none-recorded';

/** Why no bar shows: every selected country's recorded totals are zero ('zero', proven by the rosters); some are zero
 * and the rest have no recorded total ('partial'); none has one ('none-recorded'); or a group's membership was not
 * recorded then. */
export function emptyBalance(rows: readonly BalanceRow[], chosen: { tags: Set<string> | null; membersUnknown?: true }): EmptyBalance {
  if (chosen.membersUnknown) return 'members-unrecorded';
  const shown = rows.filter(row => !chosen.tags || chosen.tags.has(row.tag));
  const zero = shown.filter(row => row.sent === 0n && row.received === 0n).length;
  const wanted = chosen.tags ? chosen.tags.size : shown.length;
  return zero === 0 ? 'none-recorded' : zero === wanted ? 'zero' : 'partial';
}

export const LEND_LEASE_PARTNERS = 'lend-lease-partners';

const runs = (value: unknown) => Array.isArray(value) && value.every((run, at, all) => Array.isArray(run) && run.length === 3
  && Number.isSafeInteger(run[0]) && Number.isSafeInteger(run[1]) && run[0] <= run[1] && raw(run[2]) && (at === 0 || (all[at - 1] as number[])[1] < run[0]));
const spans = (value: unknown) => Array.isArray(value) && value.every(span => object(span) && Number.isSafeInteger(span.firstHour) && Number.isSafeInteger(span.lastHour));

export function validateLendLeasePartners(value: unknown): LendLeasePartnersReply {
  if (!object(value) || typeof value.holder !== 'string' || value.scale !== 100000 || !spans(value.known) || !Array.isArray(value.warnings)
    || !Array.isArray(value.totals) || value.totals.length !== 2 || !value.totals.every(runs)
    || !Array.isArray(value.partners) || !value.partners.every(partner => object(partner) && Number.isSafeInteger(partner.slot)
      && Array.isArray(partner.tags) && partner.tags.every(tag => object(tag) && Number.isSafeInteger(tag.firstHour) && (tag.tag === null || typeof tag.tag === 'string'))
      && Array.isArray(partner.values) && partner.values.length === 2 && partner.values.every(runs)))
    throw new Error('The lend-lease partners reply is malformed.');
  return value as unknown as LendLeasePartnersReply;
}

type ValueRuns = readonly (readonly [number, number, string])[];

/** The value a run holds at `hour`, or null where none does. */
export function valueAt(list: ValueRuns, hour: number): bigint | null {
  let low = 0, high = list.length;
  while (low < high) { const middle = (low + high) >> 1; if (list[middle][1] < hour) low = middle + 1; else high = middle; }
  const run = list[low];
  return run && run[0] <= hour ? BigInt(run[2]) : null;
}

/** A partner's tag at `hour`, or null where none holds its slot then. */
export const partnerTagAt = (partner: LendLeasePartner, hour: number): string | null =>
  partner.tags.find(tag => tag.firstHour <= hour && hour <= tag.lastHour)?.tag ?? null;

/** Each partner at `hour` in the reply's measure, largest sent plus received first (ties by slot): those with a
 * recorded value and a tag, a partner whose row left held at zero as the game drops it. */
export function partnersAt(reply: LendLeasePartnersReply, hour: number): BalanceRow[] {
  return reply.partners.flatMap(partner => {
    const tag = partnerTagAt(partner, hour), sent = valueAt(partner.values[0], hour), received = valueAt(partner.values[1], hour);
    return tag === null || sent === null || received === null ? [] : [{ slot: partner.slot, tag, sent, received }];
  }).sort((a, b) => { const x = total(a), y = total(b); return x === y ? a.slot - b.slot : x > y ? -1 : 1; });
}

/** The holder's sent and received totals over time in the reply's measure, as plotted runs. */
export function totalsOverTime(reply: LendLeasePartnersReply): { sent: Run[]; received: Run[] } {
  const plotted = (list: ValueRuns): Run[] => list.map(([firstHour, lastHour, value]) => ({ firstHour, lastHour, value: Number(BigInt(value)) / reply.scale }));
  return { sent: plotted(reply.totals[0]), received: plotted(reply.totals[1]) };
}

/** The first hour the holder has a recorded partner, or null. */
export const firstRecorded = (reply: LendLeasePartnersReply): number | null => reply.totals[0][0]?.[0] ?? null;

/** Bars draw positive amounts only; the written values stay exact and signed. */
export const barValue = (value: bigint) => Number(value > 0n ? value : 0n) / 100000;
export const amountText = (value: bigint) => formatAmount(value, PLACES, PLACES);
/** A bar label: two decimals at most, and the exact amount when that would read 0. */
export function amountLabel(value: bigint): string {
  const hundredths = rescale(value, 100000, 2);
  return hundredths === 0n && value !== 0n ? amountText(value) : formatAmount(hundredths, 2);
}

