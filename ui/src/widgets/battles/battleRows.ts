import { dayLabel } from '../../charts/draw.ts';
import { isEngineHour } from '../../dashboard/calendar.ts';
import { logDate } from '../../charts/log/logData.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { int, object } from '../../dashboard/validation.ts';
import { logStatesBy } from '../../charts/log/logStates.ts';
import type { LogRow } from '../../charts/log/logData';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';

import { placeKey } from '../../dashboard/placeNamesData.ts';
import type { BattleRow, BattleSide, BattleValue, BattleWarning, BattleWindow, PlaceNamesReply } from '../../generated/dashboard';
export type { BattleRow, BattleSide, BattleValue, BattleWarning } from '../../generated/dashboard';

export function heldAtHour<T>(read: (hour: number) => T): (hour: number) => T {
  let held: { hour: number; value: T } | null = null;
  return hour => {
    if (held === null || held.hour !== hour) held = { hour, value: read(hour) };
    return held.value;
  };
}

const indexed = new WeakMap<readonly BattleRow[], ReadonlyMap<string, BattleRow>>();
export function rowsById<T extends BattleRow>(rows: readonly T[]): ReadonlyMap<string, T> {
  let byId = indexed.get(rows);
  if (!byId) indexed.set(rows, byId = new Map(rows.map(row => [row.id, row])));
  return byId as ReadonlyMap<string, T>;
}

export interface BattleLabels {
  place: string; date: string; start: string; end: string; winner: string;
  ongoing: string; unknownPlace: string; unrecordedPlace?: string; unknownStart: string;
}

export const battlePlaceText = (row: BattleRow & { provinceId?: number | null }, labels: BattleLabels, loading = false) =>
  row.place.name ?? (row.provinceId === null ? (labels.unrecordedPlace ?? 'Province not recorded') : loading ? 'Loading…' : labels.unknownPlace);
export type BattleCountryLabel = (tag: string, hour: number) => { name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean };
export type BattleLossFormatter<T extends BattleRow = BattleRow> = (value: BattleValue, row: T, side: number, tooltip: boolean) => string;
export interface BattleMeasure<T extends BattleRow = BattleRow> {
  label: string;
  title: string;
  hint?: string;
  value: (row: T) => BattleValue;
  format?: (value: BattleValue, row: T) => string;
  tooltipFormat?: (value: BattleValue, row: T) => string;
}
export interface BattleSideSummary {
  label: string;
  countLabel: string;
  separator: string;
  unknown: string;
  formatCount: (value: BattleValue, compact: boolean) => string;
  formatSide?: (side: BattleSide, compact: boolean) => string;
}

export function battleFlagCount(count: number, width: number, flagWidth: number, gap: number, moreWidth: number): number {
  if (count === 0 || flagWidth <= 0) return 0;
  if (count * flagWidth + (count - 1) * gap <= width) return count;
  return Math.max(0, Math.min(count - 1, Math.floor((width - moreWidth) / (flagWidth + gap))));
}

export const battleDateMoment = (row: BattleRow) => row.start ?? (row.end && 'hour' in row.end ? row.end : null);
export const battleShortDate = (row: BattleRow, unknown: string) => {
  const moment = battleDateMoment(row);
  return moment === null ? unknown : `${moment.bound === 'by' ? 'by ' : ''}${logDate(moment.hour).day}`;
};

const text = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && !v.includes('\0');
const battleBound = (v: unknown) => v === null || (object(v) && typeof v.hour === 'number'
  && isEngineHour(v.hour) && (v.bound === 'exact' || v.bound === 'by'));
export function isBattleValue(v: unknown): v is BattleValue {
  return object(v) && text(v.label) && (v.value === null ? text(v.unknown)
    : typeof v.value === 'number' && Number.isFinite(v.value) && (v.atLeast === undefined || typeof v.atLeast === 'boolean'));
}
export function isBattleRow(v: unknown): v is BattleRow {
  if (!object(v) || !text(v.id) || !object(v.place) || !(v.place.name === null || text(v.place.name))
    || typeof v.place.hour !== 'number' || !isEngineHour(v.place.hour)
    || !(v.place.namedAt === undefined || typeof v.place.namedAt === 'number' && isEngineHour(v.place.namedAt) && v.place.namedAt > v.place.hour)
    || !battleBound(v.start) || !(battleBound(v.end) || (object(v.end) && text(v.end.unknown) && !('hour' in v.end)))
    || !Array.isArray(v.sides) || !v.sides.length || !v.sides.every(side => object(side) && text(side.label)
      && Array.isArray(side.countries) && side.countries.every(country => object(country) && text(country.tag)
        && typeof country.hour === 'number' && isEngineHour(country.hour))
      && !('units' in side) && (side.countriesUnknown === undefined || text(side.countriesUnknown))
      && Array.isArray(side.counts) && side.counts.every(isBattleValue) && new Set(side.counts.map(value => value.label)).size === side.counts.length
      && Array.isArray(side.losses) && side.losses.every(isBattleValue)
      && new Set([...side.counts, ...side.losses].map(value => value.label)).size === side.counts.length + side.losses.length)
    || !object(v.winner) || !(text(v.winner.unknown) ? !('side' in v.winner)
      : int(v.winner.side) && v.winner.side >= 0 && v.winner.side < v.sides.length && !('unknown' in v.winner))) return false;
  const row = v as unknown as BattleRow;
  return (row.start === null || row.start.hour <= row.place.hour)
    && (row.end === null || 'unknown' in row.end || row.end.hour >= row.place.hour)
    && (row.start === null || row.end === null || 'unknown' in row.end || row.start.hour <= row.end.hour);
}

export const battleHour = (row: BattleRow) => row.end && 'hour' in row.end ? row.end.hour : row.start?.hour ?? row.place.hour;
export const battleJumpHour = (row: BattleRow) => row.start?.hour ?? row.place.hour;
export const battleWinner = (row: BattleRow) => 'unknown' in row.winner ? row.winner.unknown : row.sides[row.winner.side].label;
export const battleDate = (bound: BattleRow['end'], unknown: string) => bound === null ? unknown
  : 'unknown' in bound ? bound.unknown : `${bound.bound === 'by' ? 'by ' : ''}${dayLabel(bound.hour)}`;
export const battleValueText = (value: BattleValue) => value.value === null ? value.unknown
  : Number.isInteger(value.value) ? usNumber.format(value.value) : String(value.value);

export function battleMeasure(row: BattleRow, label: string, unknown: string): BattleValue {
  let value = 0;
  let atLeast = false;
  const missing = new Set<string>();
  for (const side of row.sides) {
    const part = [...side.counts, ...side.losses].find(part => part.label === label);
    if (!part) missing.add(unknown);
    else if (part.value === null) missing.add(part.unknown);
    else { value += part.value; atLeast ||= part.atLeast === true; }
  }
  return missing.size ? { label, value: null, unknown: [...missing].join('; ') } : { label, value, atLeast };
}

export const battleLogOrder = (live: boolean) => <T extends BattleRow>(a: LogRow<T>, b: LogRow<T>) =>
  (live ? Number(b.item.end === null) - Number(a.item.end === null) : 0) || b.hour - a.hour || compareTags(a.item.id, b.item.id);

const battleLogRow = <T extends BattleRow>(item: T): LogRow<T> => ({ key: item.id, id: item.id, hour: battleHour(item),
  by: (item.end && 'hour' in item.end ? item.end : item.start)?.bound === 'by', item });

export function battleState(row: BattleRow, painted = false): string {
  const value = (part: BattleValue) => part.value === null ? [part.label, part.unknown] : [part.label, painted || part.value, painted || (part.atLeast ?? false)];
  return JSON.stringify([row.place.name, row.place.namedAt ?? row.place.hour, row.start, row.end, row.winner,
    row.sides.map(side => [side.label, side.countries.map(country => country.tag), side.countriesUnknown ?? null, side.counts.map(value), side.losses.map(value)])]);
}

export function battleStateRows<T extends BattleRow>(hours: readonly number[], rowsAt: (hour: number) => ReadonlyArray<T>, live: boolean,
  state: (row: T) => string = row => battleState(row), changed: (hour: number) => Iterable<string> | null = () => null,
  rowOf: (id: string, hour: number) => T | null = () => null): LogRow<T>[] {
  return logStatesBy<T>(hours, hour => rowsAt(hour).map(battleLogRow), changed, (id, hour) => { const row = rowOf(id, hour); return row && battleLogRow(row); },
    row => state(row.item), battleLogOrder(live));
}

export interface BattleChanges { readonly whole: readonly number[]; readonly battles: ReadonlyMap<number, readonly string[]> }

export function battleChanges(whole: Iterable<number>, battles: Iterable<readonly [string, Iterable<number>]>): BattleChanges {
  const byHour = new Map<number, Set<string>>();
  for (const [id, hours] of battles) for (const hour of hours) {
    const ids = byHour.get(hour);
    if (ids) ids.add(id); else byHour.set(hour, new Set([id]));
  }
  return { whole: [...new Set(whole)], battles: new Map([...byHour].map(([hour, ids]) => [hour, [...ids]])) };
}

export const battleChangeHours = (changes: BattleChanges): number[] => [...new Set([...changes.whole, ...changes.battles.keys()])].sort((a, b) => a - b);

export function heldWindowsEnd(windows: readonly BattleWindow[], held: { has(index: number): boolean }, at: number): number {
  let index = battleWindowIndex(windows, at);
  if (index < 0 || !held.has(index)) return at + 1;
  const last = Math.min(windows.length - 1, index + 1);
  while (index < last && held.has(index + 1)) index++;
  return windows[index].lastHour + 1;
}

export function battleCountryRequests(rows: readonly BattleRow[]): { tag: string; hour: number }[] {
  const wanted = new Map<number, Set<string>>();
  for (const row of rows) for (const side of row.sides) for (const country of side.countries) {
    const tags = wanted.get(country.hour) ?? new Set<string>();
    tags.add(country.tag); wanted.set(country.hour, tags);
  }
  return [...wanted].flatMap(([hour, tags]) => [...tags].map(tag => ({ tag, hour })));
}

export function battleTooltip<T extends BattleRow>(row: T, labels: BattleLabels, country: BattleCountryLabel,
  measures: readonly BattleMeasure<T>[] = [], note?: string, formatLoss?: BattleLossFormatter<T>): TooltipContent {
  const rows: TooltipRow[] = [
    { label: labels.start, value: battleDate(row.start, labels.unknownStart) },
    { label: labels.end, value: battleDate(row.end, labels.ongoing) },
    ...measures.map(measure => ({ label: measure.title, value: (measure.tooltipFormat ?? measure.format ?? battleValueText)(measure.value(row), row) })),
  ];
  for (const [index, side] of row.sides.entries()) {
    rows.push({ label: side.label, strong: true, divider: true });
    for (const part of side.countries) { const named = country(part.tag, part.hour);
      rows.push({ label: named.name, countryFlag: named, indent: true }); }
    if (side.countriesUnknown) rows.push({ label: 'Countries unknown', indent: true });
    for (const value of side.counts) rows.push({ label: value.label, value: battleValueText(value), indent: true });
    for (const value of side.losses) rows.push({ label: value.label, value: formatLoss ? formatLoss(value, row, index, true) : battleValueText(value), indent: true });
  }
  rows.push({ label: labels.winner, value: battleWinner(row), divider: true });
  return { title: battlePlaceText(row as T & { provinceId?: number | null }, labels), rows, note };
}

export function battleWindowIndex(windows: readonly BattleWindow[], at: number): number {
  for (let index = windows.length - 1; index >= 0; index--) {
    if (windows[index].firstHour <= at) return index;
  }
  return -1;
}

export function battleWindows(data: { windows: readonly BattleWindow[] } | undefined, at: number, live: boolean, visible: readonly number[] = [], from = -Infinity): number[] {
  if (!data) return [];
  const current = battleWindowIndex(data.windows, at);
  if (current < 0) return [];
  const wanted = new Set([current, ...visible.filter(index => index >= 0 && index <= current && data.windows[index]?.lastHour >= from)]);
  let oldest = current;
  for (const index of wanted) oldest = Math.min(oldest, index);
  if (oldest > 0 && data.windows[oldest - 1].lastHour >= from) wanted.add(oldest - 1);
  if (live) for (const next of [current + 1, current + 2]) if (next < data.windows.length) wanted.add(next);
  return [...wanted].sort((a, b) => a - b);
}

export function shownBattleCensusHour(windows: ReadonlyMap<number, { observed: [number, number][] }>, at: number): number | null {
  let latest: number | null = null;
  for (const reply of windows.values()) for (const [first, last] of reply.observed) {
    if (first <= at) latest = Math.max(latest ?? first, Math.min(last, at));
  }
  return latest;
}
export function battleWarnings(windows: ReadonlyMap<number, { contentWarnings: string[]; warnings: BattleWarning[] }>, first: number, last: number): string[] {
  return [...new Set([...windows.values()].flatMap(reply => [...reply.contentWarnings,
    ...reply.warnings.filter(w => w.hour >= first && w.hour <= last).map(w => `Hour ${w.hour}: ${w.reason} (${w.count})`)]))];
}
export function battlePlaceRequests(rows: readonly (BattleRow & { provinceId: number | null })[]): { provinceId: number; hour: number }[] {
  const places = new Map<string, { provinceId: number; hour: number }>();
  for (const row of rows) if (row.provinceId !== null) places.set(placeKey(row.provinceId, row.place.namedAt ?? row.place.hour), { provinceId: row.provinceId, hour: row.place.namedAt ?? row.place.hour });
  return [...places.values()].sort((a, b) => a.hour - b.hour || a.provinceId - b.provinceId);
}
export function placesLeadingWith(rows: readonly (BattleRow & { provinceId: number | null })[], rest: readonly { provinceId: number; hour: number }[]): { provinceId: number; hour: number }[] {
  const places = new Map<string, { provinceId: number; hour: number }>();
  for (const row of rows) {
    if (row.provinceId === null) continue;
    const hour = row.place.namedAt ?? row.place.hour, key = placeKey(row.provinceId, hour);
    if (!places.has(key)) places.set(key, { provinceId: row.provinceId, hour });
  }
  for (const place of rest) { const key = placeKey(place.provinceId, place.hour); if (!places.has(key)) places.set(key, place); }
  return [...places.values()];
}
export function namedBattles<T extends BattleRow & { provinceId: number | null }>(rows: readonly T[], places: PlaceNamesReply | undefined): T[] {
  const byPlace = new Map(places?.places.map(place => [placeKey(place.provinceId, place.hour), place]));
  return rows.map(row => {
    if (row.provinceId === null) return row;
    const name = byPlace.get(placeKey(row.provinceId, row.place.namedAt ?? row.place.hour))?.name ?? row.place.name;
    return name === row.place.name ? row : { ...row, place: { ...row.place, name } };
  });
}

export const rankingHour = (at: number, first: number, last: number) => Math.min(last, Math.max(first, Math.floor(at / 256) * 256));
