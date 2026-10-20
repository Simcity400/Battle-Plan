// Shared reading of the economy families' prepared histories: compact value runs
// (`[firstHour, lastHour, value]`), holder tag runs, dated variant names and the
// prepared prefix every reply carries. Playback looks values up locally.
import type { CountryGroupReply, EconomyRun, HolderTag, StockVariantName } from '../generated/dashboard';
import type { CountrySelection } from './model';
import { groupMembers, groupRuns, groupUnobserved, membersKnown } from './groups.ts';
import { object, opaqueTag } from './validation.ts';
import { pendingAt, preparedPrefix } from './preparedPrefix.ts';

/** A hexadecimal handle. */
export const hexText = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{1,16}$/.test(value);
/** A decimal integer, exact as a string. */
export const decimalText = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,39}$/.test(value);

/** Ordered, disjoint runs of decimal values. */
export function isEconomyRuns(value: unknown, check: (value: unknown) => boolean = decimalText): value is EconomyRun[] {
  if (!Array.isArray(value)) return false;
  let last = Number.NEGATIVE_INFINITY;
  for (const run of value) {
    if (!Array.isArray(run) || run.length !== 3 || !Number.isSafeInteger(run[0]) || !Number.isSafeInteger(run[1])
      || run[0] > run[1] || run[0] <= last || !check(run[2])) return false;
    last = run[1];
  }
  return true;
}

/** The run holding `hour`, by binary search; null where none does (not read). */
export function runAt(runs: readonly EconomyRun[], hour: number): EconomyRun | null {
  let low = 0, high = runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = runs[middle];
    if (hour < run[0]) high = middle - 1;
    else if (hour > run[1]) low = middle + 1;
    else return run;
  }
  return null;
}

export const valueAt = (runs: readonly EconomyRun[], hour: number): string | null => runAt(runs, hour)?.[2] ?? null;
export function bigAt(runs: readonly EconomyRun[], hour: number): bigint | null {
  const value = valueAt(runs, hour);
  return value === null ? null : BigInt(value);
}
export function numberAt(runs: readonly EconomyRun[], hour: number): number | null {
  const value = valueAt(runs, hour);
  return value === null ? null : Number(value);
}

/** The last run starting at or before `hour`; it may have ended before it. */
export function lastRunBy(runs: readonly EconomyRun[], hour: number): EconomyRun | null {
  let found: EconomyRun | null = null;
  for (const run of runs) {
    if (run[0] > hour) break;
    found = run;
  }
  return found;
}

/** A holder's tags, each from its hour until the next. */
export function isHolderTags(value: unknown): value is HolderTag[] {
  if (!Array.isArray(value)) return false;
  let last: number | null = null;
  for (const entry of value) {
    if (!object(entry) || !Number.isSafeInteger(entry.hour) || !opaqueTag(entry.tag) || (last !== null && (entry.hour as number) <= last)) return false;
    last = entry.hour as number;
  }
  return true;
}

/** The holder's tag at `hour`; null before it was first read with one. */
export function tagAt(tags: readonly HolderTag[] | undefined, hour: number): string | null {
  if (!tags) return null;
  let found: string | null = null;
  for (const entry of tags) {
    if (entry.hour > hour) break;
    found = entry.tag;
  }
  return found;
}

export function isHolderMap(value: unknown): value is Record<string, HolderTag[]> {
  return object(value) && Object.entries(value).every(([key, tags]) => /^\d+$/.test(key) && isHolderTags(tags));
}

export function isVariantNames(value: unknown): value is Record<string, StockVariantName[]> {
  return object(value) && Object.values(value).every(names => Array.isArray(names) && names.every(name => object(name)
    && Number.isSafeInteger(name.firstHour) && (name.name === null || typeof name.name === 'string') && typeof name.known === 'boolean'));
}

/** A variant's name as its latest naming read at or before `hour` held it; null
 * before the first naming read, and where that entry's names are not known or
 * content does not name it: a name never crosses an unknown interval. */
export function variantNameAt(names: readonly StockVariantName[] | undefined, hour: number): string | null {
  let found: StockVariantName | null = null;
  for (const name of names ?? []) {
    if (name.firstHour > hour) break;
    found = name;
  }
  return found && found.known ? found.name : null;
}

/** An exact fixed-point amount: whole digits and the significant fraction. */
export function fixedText(raw: string | bigint, scale: bigint, places = 5): string {
  const value = BigInt(raw), magnitude = value < 0n ? -value : value;
  const whole = magnitude / scale;
  const fraction = ((magnitude % scale) * 10n ** BigInt(places) / scale).toString().padStart(places, '0').replace(/0+$/, '');
  const grouped = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${value < 0n ? '-' : ''}${grouped}${fraction ? `.${fraction}` : ''}`;
}

/** The common reply envelope: branch, prepared prefix, warnings; `body` checks the rest. */
export function validateEconomyReply<T>(value: unknown, body: (reply: Record<string, unknown>) => boolean, what: string): T {
  if (!object(value) || typeof value.branchId !== 'string'
    || !(value.lastHour === null || Number.isSafeInteger(value.lastHour))
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string') || !body(value))
    throw new Error(`The ${what} reply is malformed.`);
  preparedPrefix(value);
  return value as T;
}

/** Whether the reply answers `hour`: it is prepared and not still pending. */
export function preparedAt(reply: { lastHour: number | null }, hour: number): boolean {
  return reply.lastHour !== null && hour <= reply.lastHour && !pendingAt(preparedPrefix(reply), hour);
}

/** A hexadecimal variant handle as the decimal the artwork product takes. */
export const variantDecimal = (hex: string): string => BigInt(`0x${hex}`).toString();

/** Why a group's rows at `hour` say nothing about its countries: its membership
 * was not recorded then ('unavailable'), or it had no members ('empty', proven).
 * Null for other selections, a group still loading, or one with members, whose
 * rows (or their absence) stand on the countries' own evidence, and while the members
 * counted are still being prepared, when neither is proven yet. */
export function groupReasonAt(selection: CountrySelection, groups: CountryGroupReply | undefined, hour: number,
  lastHour: number): { kind: 'empty' | 'unavailable'; message: string } | null {
  if (selection.kind !== 'group' || selection.group.type === 'top' || !groups) return null;
  const runs = groupRuns(groups, selection.group);
  if (!runs || !membersKnown(selection, groups, lastHour)(hour)) return null;
  const members = groupMembers(runs, selection.membership, hour, lastHour, Infinity, groupUnobserved(groups, selection.group));
  if (!members) return { kind: 'unavailable', message: 'This group’s membership was not recorded at this date.' };
  if (members.length) return null;
  return { kind: 'empty', message: selection.membership === 'hour' ? 'This group has no members at this date.'
    : selection.membership === 'end' ? 'This group has no members at the end of this campaign.' : 'This group has no members in this campaign.' };
}
