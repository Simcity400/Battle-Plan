import { usNumber } from '../../charts/numberFormat.ts';
import type { ColumnPeriod } from '../../charts/columns';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import { firstHourOfMonth, formatDate, formatDateTime, gameDate, monthAt, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { ArmyLossesReply, LossArmy, LossDesign, LossEquipment, LossManpower, LossView } from './types';

export type DesignRow = Omit<LossEquipment, 'design'> & LossDesign;

export interface ArmyLossesData extends Omit<ArmyLossesReply, 'manpower' | 'equipment' | 'designs'> {
  hour: number;
  manpower: (LossManpower & { losses: [number, number, number] | null })[];
  equipment: (DesignRow & { amountRaw: string | null })[];
}

export type Measure = 'manpower' | 'equipment';
export type ManpowerStack = 'channel' | 'armyGroup';
export type EquipmentStack = 'type' | 'armyGroup';
export type Ledger = 'combat' | 'attrition' | 'recovered' | 'enemy';

export const LEDGERS: Readonly<Record<Ledger, readonly number[]>> = { combat: [0], attrition: [1, 2, 3], recovered: [4], enemy: [5] };
export const LEDGER_LABELS: Readonly<Record<Ledger, string>> = {
  combat: 'Lost in combat', attrition: 'Lost to attrition', recovered: 'Recovered', enemy: 'Enemy losses dealt' };
export const CHANNELS = [{ key: 'ground', label: 'Ground' }, { key: 'air', label: 'Air' }, { key: 'navy', label: 'Navy' }] as const;
export const INDEPENDENT = 'independent', UNRESOLVED = 'unresolved', UNCLASSIFIED = 'unclassified', OTHER = 'other';

export function chartView(measure: Measure, ledger: Ledger): LossView {
  return measure === 'manpower' ? { kind: 'manpower' } : { kind: 'equipment', ledgers: [...LEDGERS[ledger]] };
}

export const monthView = (month: number): LossView => ({ kind: 'month', month });

const isArray = Array.isArray;
export function validateArmyLosses(value: unknown): ArmyLossesReply {
  const reply = value as ArmyLossesReply;
  if (!reply || typeof reply.tag !== 'string' || typeof reply.absent !== 'boolean'
    || !isArray(reply.armies) || !isArray(reply.manpower) || !isArray(reply.equipment) || !isArray(reply.designs)
    || !isArray(reply.emptyMonths) || !isArray(reply.unknownMonths)
    || !isArray(reply.warnings) || !reply.availability || typeof reply.availability !== 'object'
    || reply.manpower.some(row => !Number.isInteger(row.month) || typeof row.key !== 'string')
    || reply.equipment.some(row => !Number.isInteger(row.month) || typeof row.key !== 'string'
      || !Number.isInteger(row.design) || row.design < 0 || row.design >= reply.designs.length))
    throw new Error('Invalid army losses reply');
  return reply;
}

export function resolveArmyLosses(reply: ArmyLossesReply, hour: number, values: LiveValues<number>): ArmyLossesData {
  const whole = (key: string, column: string) => { const value = values.get(liveKey(key, column)); return value === undefined ? null : Math.round(value); };
  const { designs, ...rest } = reply;
  return { ...rest, hour,
    manpower: reply.manpower.map(row => {
      const losses = CHANNELS.map(channel => whole(row.key, channel.key));
      return { ...row, losses: losses.every(value => value !== null) ? losses as [number, number, number] : null };
    }),
    equipment: reply.equipment.map(({ design, ...row }) => {
      const amount = whole(row.key, 'amount');
      return { ...row, ...designs[design], amountRaw: amount === null ? null : String(amount) };
    }) };
}

export function countedArmies(reply: Pick<ArmyLossesData, 'armies'>): ReadonlyMap<string, LossArmy> {
  return new Map(reply.armies.map(army => [army.reference, army]));
}

export function armyGroupOf(army: LossArmy): string {
  return army.parent ?? (army.parentMissing === null ? INDEPENDENT : UNRESOLVED);
}

const addManpower = (left: bigint, right: bigint) => BigInt.asIntN(32, left + right);
const addExact = (left: bigint, right: bigint) => left + right;

export interface Part { key: string; label: string }
export interface MonthPart { key: string; value: number; raw: bigint }
export interface MonthPeriod extends ColumnPeriod { key: string; parts: MonthPart[]; unretained?: boolean }
export interface MonthBars { periods: MonthPeriod[]; parts: Part[]; first: number; last: number }

const plotted = (raw: bigint, measure: Measure) => measure === 'manpower' ? Number(raw) : fixedNumber(String(raw), 5);

export function measureAvailabilityReason(reply: Pick<ArmyLossesData, 'availability'>, measure: Measure, ledger: Ledger): string | null {
  if (reply.availability.armies !== null) return reply.availability.armies;
  if (measure === 'manpower') return reply.availability.manpower;
  return reply.availability.months ?? (ledger === 'enemy' ? reply.availability.enemyEquipment : reply.availability.ownEquipment);
}

export function availabilityMessage(reason: string): string {
  const trimmed = reason.trim().replace(/\.+$/, '');
  return trimmed ? `The combat log wasn’t recorded at this time: ${trimmed}.` : 'The combat log wasn’t recorded at this time.';
}

function measureUnavailable(reply: Pick<ArmyLossesData, 'availability'>, measure: Measure, ledger: Ledger): boolean {
  return measureAvailabilityReason(reply, measure, ledger) !== null;
}

export function monthBars(reply: ArmyLossesData, measure: Measure, stack: ManpowerStack | EquipmentStack, ledger: Ledger,
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
      if (!row.losses) { unknown.add(row.month); continue; }
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
      if (row.amountRaw === null) { unknown.add(row.month); continue; }
      known.add(row.month);
      const amount = BigInt(row.amountRaw);
      if (stack === 'type') {
        const key = row.archetype === null ? UNCLASSIFIED : JSON.stringify(['archetype', row.archetype]);
        if (!labels.has(key)) labels.set(key, row.archetype === null ? 'Equipment type unavailable' : row.archetypeName ?? 'Equipment name unavailable');
        add(row.month, key, amount);
      } else add(row.month, groupKey(army), amount);
    }
  }
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
  const unavailable = measureUnavailable(reply, measure, ledger);
  const periods: MonthPeriod[] = [];
  for (let month = first; month <= last; month++) {
    const firstOf = firstHourOfMonth(month), lastOf = firstHourOfMonth(month + 1) - 1;
    if (unknown.has(month) || (!known.has(month) && unavailable)) {
      periods.push({ key: String(month), firstHour: firstOf, lastHour: lastOf, parts: [], values: [], unknown: true });
      continue;
    }
    if (!known.has(month)) {
      periods.push({ key: String(month), firstHour: firstOf, lastHour: lastOf, parts: [], values: [], unknown: true, unretained: true });
      continue;
    }
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

export function formatAmount(value: number | bigint, measure: Measure): string {
  return measure === 'manpower' ? usNumber.format(value) : designAmount(String(value));
}

export function monthTooltip(period: MonthPeriod, parts: readonly Part[], measure: Measure, caption: string, playhead: number | null,
  colour: (key: string) => string): TooltipContent {
  const title = monthLabel(Number(period.key));
  if (period.unretained) return { title, subtitle: caption, rows: [{ label: 'Not in the game’s Combat Log', tone: 'muted' }] };
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

export interface MonthArmy { army: LossArmy; losses: [number, number, number] | null; designs: { name: string; amountRaw: string | null; ledger: number }[];
  unknown: number[]; amount: bigint | null }

export function monthArmies(reply: ArmyLossesData, month: number, kinds: readonly number[]): MonthArmy[] {
  const out = new Map<string, MonthArmy>();
  const entry = (army: LossArmy) => out.get(army.reference)
    ?? (out.set(army.reference, { army, losses: null, designs: [], unknown: [], amount: null }), out.get(army.reference)!);
  const armies = countedArmies(reply);
  let kept = false;
  for (const row of reply.manpower) {
    const army = armies.get(row.army);
    if (army && row.month === month) entry(army).losses = row.losses;
  }
  for (const row of reply.equipment) {
    const army = armies.get(row.army);
    if (!army || row.month !== month) continue;
    entry(army).designs.push({ name: row.name ?? 'Design name unavailable', amountRaw: row.amountRaw, ledger: row.ledger });
    kept ||= kinds.includes(row.ledger);
  }
  for (const row of reply.emptyMonths) {
    const army = armies.get(row.army);
    if (!army || row.month !== month) continue;
    entry(army);
    kept ||= kinds.includes(row.ledger);
  }
  for (const row of reply.unknownMonths) {
    const army = armies.get(row.army);
    if (army && row.month === month) entry(army).unknown.push(row.ledger);
  }
  return [...out.values()].map(row => {
    const designs = row.designs.filter(design => kinds.includes(design.ledger));
    return { ...row, amount: !kept || row.unknown.some(ledger => kinds.includes(ledger)) || designs.some(design => design.amountRaw === null) ? null
      : designs.reduce((sum, design) => sum + BigInt(design.amountRaw!), 0n) };
  })
    .sort((a, b) => a.army.ordinal - b.army.ordinal);
}

export const designAmount = (raw: string) => formatFixed(raw, 5, 0);
export const armyName = (army: LossArmy) => army.name ?? 'Army name unavailable';
