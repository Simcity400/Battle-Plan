import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { BattleWindow, LandBattleHistory, LandBattleLossRead, LandBattlesReply, LandBattlesManifestReply, LandBattlesRankingReply, PlaceNamesReply } from '../../generated/dashboard';
import type { BattleRow, BattleValue } from '../battles/battleRows';
import { battleMeasure, battleValueText, isBattleValue } from '../battles/battleRows.ts';

export interface LandBattleRow extends BattleRow {
  provinceId: number;
  windowIndex: number;
  readHour: number;
  peak: { value: number; hour: number };
  length: BattleValue;
  casualties: BattleValue;
  lossRead: { hour: number; terminal: boolean } | null;
}
const nullableText = (v: unknown) => v === null || typeof v === 'string';
const uint = (v: unknown): v is number => int(v) && v >= 0 && v <= 4294967295;
const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const moment = (v: unknown) => object(v) && hour(v.hour) && (v.bound === 'exact' || v.bound === 'by');
const peak = (v: unknown) => object(v) && int(v.value) && v.value >= 0 && hour(v.hour);
const span = (v: unknown) => object(v) && hour(v.firstHour) && hour(v.lastHour) && v.firstHour <= v.lastHour;
const warnings = (v: unknown) => Array.isArray(v) && v.every(w => object(w) && hour(w.hour) && typeof w.reason === 'string' && int(w.count) && w.count > 0);
const contentWarnings = (v: unknown) => Array.isArray(v) && v.every((warning: unknown) => typeof warning === 'string');
function lossReads(v: unknown, start: unknown): boolean {
  if (!Array.isArray(v) || !object(start) || !hour(start.hour)) return false;
  const first = start.hour;
  return v.every((read, index) => object(read) && hour(read.hour) && read.hour >= first
    && typeof read.terminal === 'boolean' && Array.isArray(read.sides) && read.sides.length === 2
    && read.sides.every(side => Array.isArray(side) && side.every(isBattleValue) && new Set(side.map(value => value.label)).size === side.length)
    && (index === 0 || read.hour >= v[index - 1].hour && !v[index - 1].terminal));
}
const histories = (v: unknown) => Array.isArray(v) && v.every(r => object(r) && typeof r.id === 'string' && uint(r.provinceId) && moment(r.start)
  && (r.end === null || moment(r.end) || object(r.end) && typeof r.end.unknown === 'string')
  && (r.endUnknownHour === null || hour(r.endUnknownHour)) && int(r.counterBase)
  && (r.priorPeak === null || peak(r.priorPeak)) && Array.isArray(r.runs) && r.runs.length > 0
  && lossReads(r.losses, r.start)
  && r.runs.every(run => span(run) && object(run) && Array.isArray(run.counts) && run.counts.length === 2 && run.counts.every(uint)
    && Array.isArray(run.countries) && run.countries.length === 2 && run.countries.every(tags => Array.isArray(tags) && tags.every(tag => typeof tag === 'string'))
    && Array.isArray(run.countriesUnknown) && run.countriesUnknown.length === 2 && run.countriesUnknown.every(value => typeof value === 'boolean')
    && int(run.duration) && run.duration >= -2147483648 && run.duration + Number(run.lastHour) - Number(run.firstHour) <= 2147483647))
  && new Set(v.map(r => r.id)).size === v.length;
export function validateLandBattlesManifest(v: unknown): LandBattlesManifestReply {
  if (!object(v) || !Array.isArray(v.windows) || !v.windows.every(span) || typeof v.available !== 'boolean' || !nullableText(v.unavailable)
    || !contentWarnings(v.contentWarnings))
    throw new Error('The land battles manifest is malformed.');
  return v as unknown as LandBattlesManifestReply;
}
export function validateLandBattlesReply(v: unknown): LandBattlesReply {
  if (!object(v) || !span(v.interval) || !int(v.window) || v.window < 0 || !histories(v.rows)
    || !Array.isArray(v.observed) || !v.observed.every(s => Array.isArray(s) && s.length === 2 && hour(s[0]) && hour(s[1]) && s[0] <= s[1])
    || !warnings(v.warnings) || !contentWarnings(v.contentWarnings)) throw new Error('The land battles reply is malformed.');
  return v as unknown as LandBattlesReply;
}
export function validateLandBattlesRanking(v: unknown): LandBattlesRankingReply {
  if (!object(v) || !histories(v.rows) || !(v.censusHour === null || hour(v.censusHour))
    || !['Casualties', 'Peak size', 'Length'].includes(String(v.effectiveMeasure)) || !warnings(v.warnings) || !contentWarnings(v.contentWarnings))
    throw new Error('The land battles ranking is malformed.');
  return v as unknown as LandBattlesRankingReply;
}

export function battleWindowIndex(windows: readonly BattleWindow[], at: number): number {
  for (let index = windows.length - 1; index >= 0; index--) {
    if (windows[index].firstHour <= at) return index;
  }
  return -1;
}

/** Hold the playhead block, Live prefetch, drawn blocks and their earlier neighbour.
 * Scrolling requests history; campaign length never determines wanted windows. */
export function battleWindows(data: LandBattlesManifestReply | undefined, at: number, live: boolean, visible: readonly number[] = [], from = -Infinity): number[] {
  if (!data) return [];
  const current = battleWindowIndex(data.windows, at);
  if (current < 0) return [];
  const wanted = new Set([current, ...visible.filter(index => index >= 0 && index <= current && data.windows[index]?.lastHour >= from)]);
  const oldest = Math.min(...wanted);
  if (oldest > 0 && data.windows[oldest - 1].lastHour >= from) wanted.add(oldest - 1);
  if (live && current + 1 < data.windows.length) wanted.add(current + 1);
  return [...wanted].sort((a, b) => a - b);
}

/** Merge in recording order, independent of network completion order. */
export function landBattleRows(windows: ReadonlyMap<number, LandBattlesReply>, at: number,
  matches: (tag: string, hour: number) => boolean, all: boolean, from: number | null = null): LandBattleRow[] {
  const histories = new Map<string, { history: LandBattleHistory; index: number }>();
  for (const [index, window] of [...windows].sort(([a], [b]) => a - b)) {
    if (window.interval.firstHour > at) continue;
    for (const history of window.rows) {
      if (history.runs.some(run => run.firstHour <= at)) histories.set(history.id, { history, index });
    }
  }
  const rows: LandBattleRow[] = [];
  for (const { history: h, index } of histories.values()) {
    const runs = h.runs.filter(run => run.firstHour <= at);
    const run = runs.at(-1);
    if (!run) continue;
    const read = Math.min(at, run.lastHour);
    const countries = run.countries.map(tags => tags.map(tag => ({ tag, hour: read })));
    if (!all && !run.countriesUnknown.some(Boolean) && !countries.some(side => side.some(country => matches(country.tag, country.hour)))) continue;
    let end: BattleRow['end'] = h.end && ('hour' in h.end ? h.end.hour <= at : h.endUnknownHour !== null && h.endUnknownHour <= at) ? h.end : null;
    const endedAt = h.end && 'hour' in h.end ? h.end.hour : h.endUnknownHour;
    if (from !== null && read < from && !(endedAt !== null && endedAt >= from && endedAt <= at)) continue;
    if (end === null && read < at) end = { unknown: 'Ending not recorded' };
    let peak = h.priorPeak ?? { value: 0, hour: h.start.hour };
    for (const run of runs) {
      const value = run.counts[0] + run.counts[1];
      if (value > peak.value || value === peak.value && run.firstHour < peak.hour) peak = { value, hour: run.firstHour };
    }
    const duration = run.duration + read - run.firstHour;
    let loss: LandBattleLossRead | undefined;
    for (let index = h.losses.length - 1; index >= 0; index--) {
      if (h.losses[index].hour <= at) { loss = h.losses[index]; break; }
    }
    const row: LandBattleRow = { id: h.id, provinceId: h.provinceId, windowIndex: index, readHour: read, start: h.start, end, place: { name: null, hour: h.start.hour }, peak,
      lossRead: loss ? { hour: loss.hour, terminal: loss.terminal } : null,
      length: { label: 'Length', value: Math.max(read - h.start.hour, duration - h.counterBase, 0), atLeast: true },
      casualties: { label: 'Casualties', value: null, unknown: 'Not in this recording' },
      sides: ['Attackers', 'Defenders'].map((label, index) => ({ label, countries: countries[index],
        ...(run.countriesUnknown[index] ? { countriesUnknown: 'Countries not recorded' } : {}),
        counts: [{ label: 'Divisions (incl. reserves)', value: run.counts[index] }], losses: loss?.sides[index] ?? [] })),
      winner: { unknown: 'Result not recorded' } };
    row.casualties = { ...battleMeasure(row, 'Manpower lost', 'Not in this recording'), label: 'Casualties' };
    rows.push(row);
  }
  return rows;
}
export function landCasualtyText(value: BattleValue, row: LandBattleRow, tooltip = false): string {
  const text = battleValueText(value);
  if (value.value === null || !row.lossRead) return text;
  const status = row.lossRead.terminal ? (tooltip ? ' terminal' : '') : ' so far';
  return `${text}${status}${tooltip ? `, as of ${formatDateTime(row.lossRead.hour)}` : ''}`;
}
export function landDivisionText(value: BattleValue, compact: boolean): string {
  if (value.value === null) return compact ? '—' : value.unknown;
  const text = battleValueText(value);
  return compact ? text : `${text} ${value.value === 1 ? 'division' : 'divisions'}`;
}
export const landCasualtiesMissing = (rows: readonly LandBattleRow[]) => rows.length > 0 && rows.every(row => row.casualties.value === null);
export const landLossText = (value: BattleValue, row: LandBattleRow, _side: number, tooltip: boolean) => landCasualtyText(value, row, tooltip);
export function casualtyRankingNote(data: LandBattlesRankingReply | undefined): string | null {
  if (data?.effectiveMeasure === 'Peak size') return 'Casualties: Not in this recording. Ranked by peak size.';
  if (data?.effectiveMeasure === 'Casualties') return 'Ranked by manpower lost on both sides. Running battles use losses so far; unknown casualties follow recorded values.';
  return null;
}
/** Ranking is computed before top N by the product, including country selection. */
export function rankedLandBattleRows(data: LandBattlesRankingReply | undefined, at: number): LandBattleRow[] {
  if (!data) return [];
  return landBattleRows(new Map([[-1, { window: 0, interval: { firstHour: at, lastHour: at }, rows: data.rows, observed: [], warnings: [], contentWarnings: data.contentWarnings }]]), at, () => true, true);
}
export function shownCensusHour(windows: ReadonlyMap<number, LandBattlesReply>, at: number): number | null {
  let latest: number | null = null;
  for (const reply of windows.values()) for (const [first, last] of reply.observed) {
    if (first <= at) latest = Math.max(latest ?? first, Math.min(last, at));
  }
  return latest;
}
export function battleWarnings(windows: ReadonlyMap<number, LandBattlesReply>, first: number, last: number): string[] {
  return [...new Set([...windows.values()].flatMap(reply => [...reply.contentWarnings,
    ...reply.warnings.filter(w => w.hour >= first && w.hour <= last).map(w => `Hour ${w.hour}: ${w.reason} (${w.count})`)]))];
}
export function validatePlaceNamesReply(v: unknown): PlaceNamesReply {
  if (!object(v) || !Array.isArray(v.places) || !v.places.every(p => object(p) && uint(p.provinceId) && hour(p.hour) && nullableText(p.name) && nullableText(p.missing))
    || new Set(v.places.map(p => placeKey(p.provinceId, p.hour))).size !== v.places.length
    || !Array.isArray(v.warnings) || !v.warnings.every(w => typeof w === 'string')) throw new Error('The place names reply is malformed.');
  return v as unknown as PlaceNamesReply;
}
export const placeKey = (provinceId: number, hour: number) => JSON.stringify([provinceId, hour]);
export function landBattlePlaceRequests(rows: readonly LandBattleRow[]): { provinceId: number; hour: number }[] {
  const places = new Map<string, { provinceId: number; hour: number }>();
  for (const row of rows) places.set(placeKey(row.provinceId, row.place.hour), { provinceId: row.provinceId, hour: row.place.hour });
  return [...places.values()].sort((a, b) => a.hour - b.hour || a.provinceId - b.provinceId);
}
export function namedLandBattles(rows: readonly LandBattleRow[], places: PlaceNamesReply | undefined): LandBattleRow[] {
  const byPlace = new Map(places?.places.map(place => [placeKey(place.provinceId, place.hour), place]));
  return rows.map(row => {
    const name = byPlace.get(placeKey(row.provinceId, row.place.hour))?.name ?? row.place.name;
    return name === row.place.name ? row : { ...row, place: { ...row.place, name } };
  });
}
