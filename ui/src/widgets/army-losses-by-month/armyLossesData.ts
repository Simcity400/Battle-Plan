import { usNumber } from '../../charts/numberFormat.ts';
import type { ColumnPeriod } from '../../charts/columns';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import { firstHourOfMonth, formatDate, formatDateTime, gameDate, monthAt, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import type { ArmyLossesReply, LossArmy } from '../../generated/dashboard';

export type Measure = 'manpower' | 'equipment';
export type ManpowerStack = 'channel' | 'armyGroup';
export type EquipmentStack = 'type' | 'armyGroup';
export type Ledger = 'combat' | 'attrition' | 'recovered' | 'enemy';

/** The game's Combat Log equipment ledgers, by what each heading adds up: the three
 * attrition modes partition attrition and add together; recovery and losses dealt
 * stand alone and are never netted against losses. */
export const LEDGERS: Readonly<Record<Ledger, readonly number[]>> = { combat: [0], attrition: [1, 2, 3], recovered: [4], enemy: [5] };
export const LEDGER_LABELS: Readonly<Record<Ledger, string>> = {
  combat: 'Lost in combat', attrition: 'Lost to attrition', recovered: 'Recovered', enemy: 'Enemy losses dealt' };
export const CHANNELS = [{ key: 'ground', label: 'Ground' }, { key: 'air', label: 'Air' }, { key: 'navy', label: 'Navy' }] as const;
export const INDEPENDENT = 'independent', UNRESOLVED = 'unresolved', UNCLASSIFIED = 'unclassified', OTHER = 'other';

const isArray = Array.isArray;
export function validateArmyLosses(value: unknown): ArmyLossesReply {
  const reply = value as ArmyLossesReply;
  if (!reply || typeof reply.branchId !== 'string' || !Number.isInteger(reply.hour) || typeof reply.tag !== 'string' || typeof reply.absent !== 'boolean'
    || !isArray(reply.armies) || !isArray(reply.manpower) || !isArray(reply.equipment) || !isArray(reply.emptyMonths) || !isArray(reply.unknownMonths)
    || !isArray(reply.warnings)
    || reply.manpower.some(row => !Number.isInteger(row.month) || !isArray(row.losses) || row.losses.length !== 3 || !row.losses.every(Number.isInteger))
    || reply.equipment.some(row => !Number.isInteger(row.month) || typeof row.amountRaw !== 'string'))
    throw new Error('Invalid army losses reply');
  return reply;
}

/** The armies whose logs make the country's total: every distinct ordinary army of
 * the country once, whether it has an army group or not. Army groups keep no log
 * of their own, so no parent is ever added beside its armies. */
export function countedArmies(reply: ArmyLossesReply): ReadonlyMap<string, LossArmy> {
  return new Map(reply.armies.map(army => [army.reference, army]));
}

/** The army group an army belongs to now, the independent armies, or the explicit unresolved bucket. */
export function armyGroupOf(army: LossArmy): string {
  return army.parent ?? (army.parentMissing === null ? INDEPENDENT : UNRESOLVED);
}

/** Manpower adds as the game's signed 32-bit counters add: wrapping, never widened. */
const addManpower = (left: bigint, right: bigint) => BigInt.asIntN(32, left + right);
const addExact = (left: bigint, right: bigint) => left + right;

export interface Part { key: string; label: string }
/** A part's exact amount (whole people, or the equipment's five-decimal fixed point as
 * stored) and the number it plots at. */
export interface MonthPart { key: string; value: number; raw: bigint }
/** A month's column: its parts' plotted amounts as the column's bands, in part order. */
export interface MonthPeriod extends ColumnPeriod { key: string; parts: MonthPart[] }
export interface MonthBars { periods: MonthPeriod[]; parts: Part[]; first: number; last: number }

/** The number an exact amount plots at. */
const plotted = (raw: bigint, measure: Measure) => measure === 'manpower' ? Number(raw) : fixedNumber(String(raw), 5);

/** Each calendar month from the first to the last hour, stacked by the chosen parts.
 * A month with no retained entry is unknown: the game kept nothing for it (pruned, or
 * never written), which is not a known zero. So is a month any counted army's ledger
 * holds unwitnessed amounts for: the others alone are not its total. `limit` folds the
 * smallest parts into Other. Amounts add exactly; only plotting makes them numbers. */
export function monthBars(reply: ArmyLossesReply, measure: Measure, stack: ManpowerStack | EquipmentStack, ledger: Ledger,
  firstHour: number, lastHour: number, limit: number): MonthBars {
  const armies = countedArmies(reply);
  const first = monthOf(firstHour), last = monthOf(lastHour);
  const values = new Map<number, Map<string, bigint>>();
  const known = new Set<number>(), unknown = new Set<number>();
  const labels = new Map<string, string>();
  const sum = measure === 'manpower' ? addManpower : addExact;
  const add = (month: number, key: string, value: bigint) => {
    const parts = values.get(month) ?? new Map<string, bigint>();
    parts.set(key, sum(parts.get(key) ?? 0n, value));
    values.set(month, parts);
  };
  const groupKey = (army: LossArmy) => {
    const key = armyGroupOf(army);
    if (!labels.has(key)) labels.set(key, key === INDEPENDENT ? 'Independent armies' : key === UNRESOLVED ? 'Army group not recorded'
      : army.parentName ?? 'Army group name unavailable');
    return key;
  };
  if (measure === 'manpower') {
    for (const row of reply.manpower) {
      const army = armies.get(row.army);
      if (!army) continue;
      known.add(row.month);
      const losses = row.losses.map(value => BigInt(value));
      if (stack === 'channel') CHANNELS.forEach((channel, index) => add(row.month, channel.key, losses[index]));
      else add(row.month, groupKey(army), losses.reduce(addManpower, 0n));
    }
    for (const channel of CHANNELS) labels.set(channel.key, channel.label);
  } else {
    const kinds = LEDGERS[ledger];
    for (const month of reply.emptyMonths) if (kinds.includes(month.ledger) && armies.has(month.army)) known.add(month.month);
    for (const month of reply.unknownMonths) if (kinds.includes(month.ledger) && armies.has(month.army)) unknown.add(month.month);
    for (const row of reply.equipment) {
      const army = armies.get(row.army);
      if (!army || !kinds.includes(row.ledger)) continue;
      known.add(row.month);
      const amount = BigInt(row.amountRaw);
      if (stack === 'type') {
        const key = row.archetype ?? UNCLASSIFIED;
        if (!labels.has(key)) labels.set(key, row.archetype === null ? 'Equipment type unavailable' : row.archetypeName ?? 'Equipment name unavailable');
        add(row.month, key, amount);
      } else add(row.month, groupKey(army), amount);
    }
  }
  // Order parts by their total over the shown months, largest first; fold the rest.
  const totals = new Map<string, bigint>();
  for (const [month, parts] of values) if (month >= first && month <= last)
    for (const [key, value] of parts) totals.set(key, (totals.get(key) ?? 0n) + (value < 0n ? -value : value));
  const ordered = measure === 'manpower' && stack === 'channel' ? CHANNELS.map(channel => channel.key as string)
    : [...totals.keys()].sort((a, b) => { const left = totals.get(a)!, right = totals.get(b)!; return left > right ? -1 : left < right ? 1 : a.localeCompare(b); });
  const kept = ordered.length > limit ? ordered.slice(0, Math.max(0, limit - 1)) : ordered;
  const folded = new Set(ordered.slice(kept.length));
  const parts: Part[] = kept.map(key => ({ key, label: labels.get(key) ?? key }));
  if (folded.size) parts.push({ key: OTHER, label: 'Other' });
  const part = (key: string, raw: bigint): MonthPart => ({ key, value: plotted(raw, measure), raw });
  const periods: MonthPeriod[] = [];
  for (let month = first; month <= last; month++) {
    const firstOf = firstHourOfMonth(month), lastOf = firstHourOfMonth(month + 1) - 1;
    if (!known.has(month) || unknown.has(month)) { periods.push({ key: String(month), firstHour: firstOf, lastHour: lastOf, parts: [], values: [], unknown: true }); continue; }
    const shown = values.get(month) ?? new Map<string, bigint>();
    let other = 0n;
    for (const [key, value] of shown) if (folded.has(key)) other = sum(other, value);
    const monthParts = [...kept.map(key => part(key, shown.get(key) ?? 0n)), ...(folded.size ? [part(OTHER, other)] : [])];
    periods.push({ key: String(month), firstHour: firstOf, lastHour: lastOf, parts: monthParts, values: monthParts.map(item => item.value) });
  }
  return { periods, parts, first, last };
}

export function monthOf(hour: number): number {
  const date = gameDate(hour);
  return date ? monthIndex(date) : 0;
}

export function monthLabel(month: number): string {
  const { year, month: number } = monthAt(month);
  return `${monthNames[number - 1]} ${year}`;
}

/** Whole people, or whole items of a five-decimal amount, from an exact value. */
export function formatAmount(value: number | bigint, measure: Measure): string {
  return measure === 'manpower' ? usNumber.format(value) : designAmount(String(value));
}

/** A month's parts, its exact total and whether the playback date is inside it. */
export function monthTooltip(period: MonthPeriod, parts: readonly Part[], measure: Measure, caption: string, playhead: number | null,
  colour: (key: string) => string): TooltipContent {
  const title = monthLabel(Number(period.key));
  if (period.unknown) return { title, subtitle: caption, rows: [{ label: 'Not recorded', tone: 'muted' }] };
  const labels = new Map(parts.map(part => [part.key, part.label]));
  const rows: TooltipRow[] = period.parts.map(part => ({ label: labels.get(part.key) ?? part.key, swatch: colour(part.key), value: formatAmount(part.raw, measure) }));
  const total = period.parts.reduce((sum, part) => (measure === 'manpower' ? addManpower : addExact)(sum, part.raw), 0n);
  rows.push({ label: 'Total', strong: true, divider: true, value: formatAmount(total, measure) });
  if (playhead !== null && playhead >= period.firstHour && playhead < period.lastHour) {
    const date = gameDate(playhead);
    rows.push({ label: 'Partial month', value: date ? `to ${formatDate(date)}` : formatDateTime(playhead), tone: 'muted' });
  }
  return { title, subtitle: caption, rows };
}

/** One army's retained month: its channels and its ledgers' designs, for the drill. */
export interface MonthArmy { army: LossArmy; losses: [number, number, number] | null; designs: { name: string; amountRaw: string; ledger: number }[] }

export function monthArmies(reply: ArmyLossesReply, month: number): MonthArmy[] {
  const out = new Map<string, MonthArmy>();
  const entry = (army: LossArmy) => out.get(army.reference) ?? (out.set(army.reference, { army, losses: null, designs: [] }), out.get(army.reference)!);
  const armies = countedArmies(reply);
  for (const row of reply.manpower) {
    const army = armies.get(row.army);
    if (army && row.month === month) entry(army).losses = [row.losses[0], row.losses[1], row.losses[2]];
  }
  for (const row of reply.equipment) {
    const army = armies.get(row.army);
    if (army && row.month === month) entry(army).designs.push({ name: row.name ?? 'Design name unavailable', amountRaw: row.amountRaw, ledger: row.ledger });
  }
  return [...out.values()].sort((a, b) => a.army.ordinal - b.army.ordinal);
}

export const designAmount = (raw: string) => formatFixed(raw, 5, 0);
export const armyName = (army: LossArmy) => army.name ?? 'Army name unavailable';
