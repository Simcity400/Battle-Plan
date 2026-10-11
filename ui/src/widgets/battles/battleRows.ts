import { dayLabel } from '../../charts/draw.ts';
import { isEngineHour } from '../../dashboard/calendar.ts';
import { logDate } from '../../charts/log/logData.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { LogRow } from '../../charts/log/logData';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';

import { placeKey } from '../../dashboard/placeNamesData.ts';
import type { BattleRow, BattleValue, BattleWindow, BattleWarning, PlaceNamesReply } from '../../generated/dashboard';
export type { BattleRow, BattleSide, BattleValue, BattleWindow, BattleWarning } from '../../generated/dashboard';

export interface BattleLabels {
  place: string; date: string; start: string; end: string; winner: string;
  ongoing: string; unknownPlace: string; unknownStart: string;
}
export type BattleCountryLabel = (tag: string, hour: number) => { name: string; flag?: string; flagReason?: string; flagPending?: boolean };
export type BattleLossFormatter<T extends BattleRow = BattleRow> = (value: BattleValue, row: T, side: number, tooltip: boolean) => string;
export interface BattleMeasure<T extends BattleRow = BattleRow> {
  /** The caller's count/loss label is the stable key, never its array index. */
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
}

/** Display capacity only: undisplayed flags remain in the row's full tooltip. */
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
  : `${value.atLeast ? '≥ ' : ''}${Number.isInteger(value.value) ? usNumber.format(value.value) : String(value.value)}`;

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

export function battleLogRows<T extends BattleRow>(rows: readonly T[], live: boolean): LogRow<T>[] {
  return rows.map(item => ({ key: item.id, id: item.id, hour: battleHour(item),
    by: (item.end && 'hour' in item.end ? item.end : item.start)?.bound === 'by', item }))
    .sort((a, b) => (live ? Number(b.item.end === null) - Number(a.item.end === null) : 0)
      || b.hour - a.hour || compareTags(a.item.id, b.item.id));
}

/** Each side may have countries dated differently; never flatten those hours. */
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
  return { title: row.place.name ?? labels.unknownPlace, rows, note };
}

export function battleWindowIndex(windows: readonly BattleWindow[], at: number): number {
  for (let index = windows.length - 1; index >= 0; index--) {
    if (windows[index].firstHour <= at) return index;
  }
  return -1;
}

/** Hold the playhead block, Live prefetch, drawn blocks and their earlier neighbour.
 * Scrolling requests history; campaign length never determines wanted windows. */
export function battleWindows(data: { windows: readonly BattleWindow[] } | undefined, at: number, live: boolean, visible: readonly number[] = [], from = -Infinity): number[] {
  if (!data) return [];
  const current = battleWindowIndex(data.windows, at);
  if (current < 0) return [];
  const wanted = new Set([current, ...visible.filter(index => index >= 0 && index <= current && data.windows[index]?.lastHour >= from)]);
  let oldest = current;
  for (const index of wanted) oldest = Math.min(oldest, index);
  if (oldest > 0 && data.windows[oldest - 1].lastHour >= from) wanted.add(oldest - 1);
  if (live && current + 1 < data.windows.length) wanted.add(current + 1);
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
export function battlePlaceRequests(rows: readonly (BattleRow & { provinceId: number })[]): { provinceId: number; hour: number }[] {
  const places = new Map<string, { provinceId: number; hour: number }>();
  for (const row of rows) places.set(placeKey(row.provinceId, row.place.namedAt ?? row.place.hour), { provinceId: row.provinceId, hour: row.place.namedAt ?? row.place.hour });
  return [...places.values()].sort((a, b) => a.hour - b.hour || a.provinceId - b.provinceId);
}
export function namedBattles<T extends BattleRow & { provinceId: number }>(rows: readonly T[], places: PlaceNamesReply | undefined): T[] {
  const byPlace = new Map(places?.places.map(place => [placeKey(place.provinceId, place.hour), place]));
  return rows.map(row => {
    const name = byPlace.get(placeKey(row.provinceId, row.place.namedAt ?? row.place.hour))?.name ?? row.place.name;
    return name === row.place.name ? row : { ...row, place: { ...row.place, name } };
  });
}
