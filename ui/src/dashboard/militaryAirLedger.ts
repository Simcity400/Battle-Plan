
type LedgerPoint = MilitaryAirPart['points'][number];
import { countryFromKey, isSlotKey } from './countryKey.ts';
import { causeLabel } from '../widgets/bombing-damage/militaryAirAt.ts';
import { isOrderedSpans, runAt } from '../charts/series.ts';
import type { Span } from '../charts/series';
import { formatAmount, formatRate } from '../charts/valueAxis.ts';
import { object, opaqueTag, signed32, unsigned32 } from './validation.ts';
import type { MilitaryAirCountry, MilitaryAirOpening, MilitaryAirPart, MilitaryAirReply } from '../generated/dashboard';
import type { SunburstText } from '../charts/sunburst/layout';

export type MilitaryAirLedger = Omit<MilitaryAirReply, 'window' | 'notes' | 'countedFrom' | 'openings' | 'unjoined' | 'causes'> & Partial<Pick<MilitaryAirReply, 'causes'>>
  & { countedFrom?: number | null; openings?: MilitaryAirOpening[] };

const signed128 = (value: bigint) => value >= -(1n << 127n) && value < (1n << 127n);
const exact128 = (value: unknown): value is string => typeof value === 'string' && /^(?:0|-?[1-9]\d*)$/.test(value)
  && (value.length < 39 || signed128(BigInt(value)));
const optional = <T>(value: unknown, valid: (value: unknown) => value is T): value is T | null => value === null || valid(value);
const text = (value: unknown): value is string => typeof value === 'string';
const bool = (value: unknown): value is boolean => typeof value === 'boolean';
const within = (span: Span, spans: readonly Span[]) => (runAt(spans, span.firstHour)?.lastHour ?? -Infinity) >= span.lastHour;

const count = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

function isOpenings(value: unknown, family: unknown): value is MilitaryAirOpening[] {
  if (!Array.isArray(value)) return false;
  const keys = new Set<string>();
  for (const opening of value as unknown[]) {
    if (!object(opening) || !optional(opening.identity, text) || opening.identity === '' || !bool(opening.dealt) || !unsigned32(opening.cause)
      || (family === 'airLosses' ? !bool(opening.opponentKnown) : opening.opponentKnown !== null) || !exact128(opening.total)) return false;
    const key = JSON.stringify([opening.identity, opening.dealt, opening.cause, opening.opponentKnown]);
    if (keys.has(key)) return false;
    keys.add(key);
  }
  return true;
}

function isMilitaryAirReply(value: unknown, windowed = false): value is MilitaryAirLedger {
  if (!object(value) || !['airLosses', 'bombing'].includes(String(value.family)) || !signed32(value.firstHour) || !signed32(value.lastHour)
    || typeof value.available !== 'boolean' || !isOrderedSpans(value.observed) || !isOrderedSpans(value.incomplete)
    || !Array.isArray(value.countries) || !Array.isArray(value.parts) || !Array.isArray(value.regions) || !Array.isArray(value.buildings)
    || !Array.isArray(value.warnings) || !value.warnings.every(text)) return false;
  if (windowed && (!unsigned32(value.window) || !Array.isArray(value.notes) || !value.notes.every(note => object(note) && text(note.reason) && count(note.events)))) return false;
  if ((windowed || value.countedFrom !== undefined) && !optional(value.countedFrom, signed32)) return false;
  if ((windowed || value.openings !== undefined) && !isOpenings(value.openings, value.family)) return false;
  if (value.firstHour > value.lastHour && !(windowed && !value.observed.length && !value.countries.length && !value.parts.length)) return false;
  const observed = value.observed, first = value.firstHour, lastHour = value.lastHour;
  const reachesBack = (span: Span, index: number) => index === 0 && span.lastHour >= first && typeof value.countedFrom === 'number' && span.firstHour >= value.countedFrom;
  if (observed.some((span, index) => (span.firstHour < first && !reachesBack(span, index)) || span.lastHour > lastHour) || value.available !== (observed.length > 0)
    || !value.incomplete.every(span => within(span, observed))) return false;
  const countries = new Map<string, Map<string, Span[]>>();
  for (const country of value.countries) {
    if (!object(country) || !opaqueTag(country.tag) || !text(country.identity) || !country.identity || !isOrderedSpans(country.present)) return false;
    const membership: unknown = country.membership;
    if (!optional(membership, isOrderedSpans)) return false;
    const identities = countries.get(country.tag) ?? new Map<string, Span[]>();
    const present = country.present;
    if (identities.has(country.identity) || !present.every(span => within(span, observed))
      || (membership as Span[] | null | undefined)?.some(span => !within(span, present))) return false;
    identities.set(country.identity, present);
    countries.set(country.tag, identities);
  }
  const keys = new Set<string>();
  for (const part of value.parts) {
    if (!object(part) || !optional(part.tag, opaqueTag) || !optional(part.identity, text) || (part.tag === null) !== (part.identity === null)
      || typeof part.dealt !== 'boolean' || !optional(part.cause, unsigned32) || !optional(part.opponentKnown, bool) || !optional(part.buildingId, unsigned32)
      || !optional(part.buildingToken, text) || !optional(part.state, unsigned32) || !Array.isArray(part.points)) return false;
    if (value.family === 'airLosses' ? part.cause === null || part.opponentKnown === null || part.buildingId !== null || part.buildingToken !== null || part.state !== null
      : part.opponentKnown !== null || (part.cause === null) !== (part.buildingId === null) || (part.buildingId === null) !== (part.state === null)) return false;
    const key = JSON.stringify([part.tag, part.identity, part.dealt, part.cause, part.opponentKnown, part.buildingId, part.buildingToken, part.state]);
    if (keys.has(key)) return false;
    keys.add(key);
    const present = part.tag === null ? observed : countries.get(part.tag)?.get(part.identity as string);
    if (!present) return false;
    let last: number | null = null, read: number | null = null;
    for (const point of part.points as unknown[]) {
      if (!Array.isArray(point) || point.length !== 3) return false;
      const [hour, readHour, total] = point as unknown[];
      if (!signed32(hour) || !signed32(readHour) || hour > readHour || !exact128(total)
        || read !== null && (readHour < read || readHour === read && last !== null && hour <= last)
        || !runAt(present, readHour)) return false;
      last = hour; read = readHour;
    }
  }
  for (const names of [value.regions, value.buildings]) {
    const ids = new Set<string>();
    for (const name of names) {
      if (!object(name) || !text(name.id) || !name.id || !optional(name.name, text) || !optional(name.missing, text)
        || (name.name === null) !== (name.missing !== null) || ids.has(name.id)) return false;
      ids.add(name.id);
    }
  }
  return true;
}

export function validateMilitaryAirWindow(value: unknown): MilitaryAirReply {
  if (!isMilitaryAirReply(value, true)) throw new Error('Invalid military air ledger reply');
  return value as unknown as MilitaryAirReply;
}

export interface LedgerTotal { readHour: number; nativeHour: number | null; dealt: bigint; taken: bigint }
export interface LedgerSeries { country: MilitaryAirCountry; points: LedgerTotal[]; parts: MilitaryAirPart[]; openings: MilitaryAirOpening[] }
export interface LedgerIndex { reply: MilitaryAirLedger; series: LedgerSeries[]; byTag: ReadonlyMap<string, LedgerSeries[]>; unknown: LedgerTotal[] }
const sharedIndices = new WeakMap<MilitaryAirLedger, LedgerIndex>();
const keptTotals = new WeakMap<MilitaryAirLedger, ReadonlyMap<string | null, LedgerTotal[]>>();

export function keepLedgerTotals(ledger: MilitaryAirLedger, totals: ReadonlyMap<string | null, LedgerTotal[]>) {
  keptTotals.set(ledger, totals);
}

export function openingTotals(openings: readonly MilitaryAirOpening[] = []): Map<string | null, { dealt: bigint; taken: bigint }> {
  const out = new Map<string | null, { dealt: bigint; taken: bigint }>();
  for (const opening of openings) {
    const held = out.get(opening.identity) ?? { dealt: 0n, taken: 0n };
    held[opening.dealt ? 'dealt' : 'taken'] += BigInt(opening.total);
    out.set(opening.identity, held);
  }
  return out;
}

export function carriedTotals(totals: readonly LedgerTotal[], carried: { dealt: bigint; taken: bigint } | undefined, firstHour: number): LedgerTotal[] {
  if (!carried || carried.dealt === 0n && carried.taken === 0n) return totals as LedgerTotal[];
  return [{ readHour: firstHour - 1, nativeHour: null, ...carried },
    ...totals.map(total => ({ ...total, dealt: total.dealt + carried.dealt, taken: total.taken + carried.taken }))];
}

export function ledgerTotals(parts: readonly MilitaryAirPart[]): LedgerTotal[] {
  return safeLedgerTotals(parts) ?? exactLedgerTotals(parts);
}

const SAFE_TOTAL = /^-?\d{1,15}$/;

function safeLedgerTotals(parts: readonly MilitaryAirPart[]): LedgerTotal[] | null {
  const deltas = new Map<number, { dealt: number; taken: number; nativeHour: number }>();
  for (const part of parts) {
    const side = part.dealt ? 'dealt' : 'taken';
    let before = 0;
    for (const [hour, readHour, total] of part.points) {
      if (!SAFE_TOTAL.test(total)) return null;
      const current = Number(total), delta = current - before;
      before = current;
      const row = deltas.get(readHour) ?? { dealt: 0, taken: 0, nativeHour: hour };
      row[side] += delta;
      if (!Number.isSafeInteger(row[side])) return null;
      row.nativeHour = Math.max(row.nativeHour, hour);
      deltas.set(readHour, row);
    }
  }
  let dealt = 0, taken = 0;
  const totals: LedgerTotal[] = [];
  for (const [readHour, delta] of [...deltas].sort(([a], [b]) => a - b)) {
    dealt += delta.dealt; taken += delta.taken;
    if (!Number.isSafeInteger(dealt) || !Number.isSafeInteger(taken)) return null;
    totals.push({ readHour, nativeHour: delta.nativeHour, dealt: BigInt(dealt), taken: BigInt(taken) });
  }
  return totals;
}

function exactLedgerTotals(parts: readonly MilitaryAirPart[]): LedgerTotal[] {
  const deltas = new Map<number, { dealt: bigint; taken: bigint; nativeHour: number }>();
  for (const part of parts) {
    let before = 0n;
    for (const [hour, readHour, total] of part.points) {
      const current = BigInt(total), delta = current - before;
      before = current;
      const row = deltas.get(readHour) ?? { dealt: 0n, taken: 0n, nativeHour: hour };
      row[part.dealt ? 'dealt' : 'taken'] += delta;
      row.nativeHour = Math.max(row.nativeHour, hour);
      deltas.set(readHour, row);
    }
  }
  let dealt = 0n, taken = 0n;
  return [...deltas].sort(([a], [b]) => a - b).map(([readHour, delta]) => {
    dealt += delta.dealt; taken += delta.taken;
    return { readHour, nativeHour: delta.nativeHour, dealt, taken };
  });
}

export function militaryAirIndex(reply: MilitaryAirLedger): LedgerIndex {
  const shared = sharedIndices.get(reply);
  if (shared) return shared;
  const byCountry = new Map<string, MilitaryAirPart[]>(), unknown: MilitaryAirPart[] = [];
  for (const part of reply.parts) {
    if (part.tag === null) { unknown.push(part); continue; }
    const key = part.identity!, parts = byCountry.get(key) ?? [];
    parts.push(part); byCountry.set(key, parts);
  }
  const opened = new Map<string | null, MilitaryAirOpening[]>();
  for (const opening of reply.openings ?? []) opened.set(opening.identity, [...(opened.get(opening.identity) ?? []), opening]);
  const kept = keptTotals.get(reply), carried = kept ? null : openingTotals(reply.openings);
  const totalsOf = (identity: string | null, parts: readonly MilitaryAirPart[]) =>
    kept?.get(identity) ?? (kept ? [] : carriedTotals(ledgerTotals(parts), carried!.get(identity), reply.firstHour));
  const byHolder = new Map<string, LedgerTotal[]>(), byTag = new Map<string, LedgerSeries[]>();
  const series = reply.countries.map(country => {
    const parts = byCountry.get(country.identity) ?? [];
    let points = byHolder.get(country.identity);
    if (!points) { points = totalsOf(country.identity, parts); byHolder.set(country.identity, points); }
    const row = { country, parts, points, openings: opened.get(country.identity) ?? [] };
    const aliases = byTag.get(country.tag) ?? [];
    aliases.push(row); byTag.set(country.tag, aliases);
    return row;
  });
  const index = { reply, series, byTag, unknown: totalsOf(null, unknown) };
  sharedIndices.set(reply, index);
  return index;
}

function at<T>(points: readonly T[], hour: number, date: (point: T) => number): T | null {
  let low = 0, high = points.length;
  while (low < high) { const mid = Math.floor((low + high) / 2); if (date(points[mid]) <= hour) low = mid + 1; else high = mid; }
  return low ? points[low - 1] : null;
}
const empty = (hour: number): LedgerTotal => ({ readHour: hour, nativeHour: null, dealt: 0n, taken: 0n });
export const ledgerComplete = (spans: readonly Span[], from: number, hour: number, incomplete: readonly Span[] = []) => {
  const span = runAt(spans, hour);
  return from <= hour && span !== null && span.firstHour <= from && !incomplete.some(read => read.firstHour <= hour && read.lastHour >= from);
};
export const ledgerDifference = (points: readonly LedgerTotal[], from: number, hour: number): LedgerTotal => {
  const last = at(points, hour, point => point.readHour) ?? empty(hour);
  const before = at(points, from - 1, point => point.readHour) ?? empty(from);
  return { ...last, readHour: hour, dealt: last.dealt - before.dealt, taken: last.taken - before.taken };
};

export interface LedgerRow extends LedgerTotal { tag: string; identity: string; segment: number; complete: boolean; series: LedgerSeries }
export function militaryAirAt(index: LedgerIndex, from: number, hour: number): LedgerRow[] {
  if (!runAt(index.reply.observed, hour)) return [];
  return index.series.flatMap(series => {
    const span = runAt(series.country.present, hour);
    if (!span) return [];
    return [{ ...ledgerDifference(series.points, from, hour), tag: series.country.tag, identity: series.country.identity,
      segment: span.firstHour, complete: ledgerComplete(index.reply.observed, from, hour, index.reply.incomplete), series }];
  });
}

export function ledgerNames(key: string, country: Pick<MilitaryAirCountry, 'tag' | 'identity'>): boolean {
  const slot = countryFromKey(key);
  return typeof slot === 'number' ? country.identity.startsWith(`${slot}:`) : country.tag === key;
}

export function ledgerCountryAt(index: LedgerIndex, tag: string, from: number, hour: number, identity?: string): LedgerRow | null {
  const named = isSlotKey(tag) ? index.series.filter(series => ledgerNames(tag, series.country)) : index.byTag.get(tag) ?? [];
  const chosen = identity ?? named.reduce<MilitaryAirCountry | null>((first, series) => {
    const country = series.country;
    return !first || (country.present[0]?.firstHour ?? Infinity) < (first.present[0]?.firstHour ?? Infinity) ? country : first;
  }, null)?.identity;
  const rows = militaryAirAt(index, from, hour).filter(row => row.identity === chosen);
  return rows.find(row => row.tag === tag) ?? rows[0] ?? null;
}

export function ledgerHeaderTag(index: LedgerIndex, row: LedgerRow | null, tag: string, hour: number, identity: string | undefined,
  selection: { tag: string; identity: string } | null): string | null {
  if (!row) return identity === undefined ? tag : null;
  const pinned = !selection || selection.identity === identity
    && index.byTag.get(selection.tag)?.some(series => series.country.identity === identity)
    && index.series.filter(series => series.country.identity === identity && runAt(series.country.present, hour)).length === 1;
  return pinned && index.byTag.get(row.tag)?.filter(series => runAt(series.country.present, hour)).length === 1 ? row.tag : null;
}

export function ledgerPinnedIdentity(value: unknown, campaign: string, branch: string, tag: string): string | undefined {
  if (typeof value !== 'string') return undefined;
  try {
    const saved: unknown = JSON.parse(value);
    return Array.isArray(saved) && saved.length === 4 && saved[0] === campaign && saved[1] === branch && saved[2] === tag
      && typeof saved[3] === 'string' && saved[3] ? saved[3] : undefined;
  } catch { return undefined; }
}

export const ledgerScale = (family: MilitaryAirLedger['family']) => family === 'airLosses' ? 100_000 : 10_000_000;
const ledgerDigits = (family: MilitaryAirLedger['family']) => family === 'airLosses' ? 5 : 7;
export function ledgerLabel(raw: bigint | null, family: MilitaryAirLedger['family']): string {
  if (raw === null) return 'Not recorded';
  const text = formatAmount(raw, ledgerDigits(family));
  return text === '0' && raw !== 0n ? formatRate(raw, ledgerDigits(family)) : text;
}
export const BOMBING_SUNBURST_TEXT: SunburstText = { format: (_value, exact) => ledgerLabel(exact, 'bombing'), unit: 'building levels', valueLabel: 'Damage', countLabel: 'Locations' };

export function partAmount(part: MilitaryAirPart, from: number, hour: number): bigint {
  const total = (point: LedgerPoint | null) => point ? BigInt(point[2]) : 0n;
  return total(at(part.points, hour, point => point[1])) - total(at(part.points, from - 1, point => point[1]));
}

export function ledgerCauseName(reply: Pick<MilitaryAirLedger, 'causes'> | undefined, cause: number): string {
  const named = reply?.causes?.find(item => item.cause === cause);
  return named ? causeLabel(named) : `Cause ${cause}`;
}
