import type { PlaybackIntervals } from '../../charts/playbackIntervals';
import { playbackRows, spanChanges } from '../../charts/playbackIntervals.ts';
import { countryMatchChangeHours } from '../../dashboard/groups.ts';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import type { BattleRow, BattleSide, BattleValue, BattleWindow, NavalBattleHistory, NavalBattleRun, NavalBattlesReply,
  NavalBattlesManifestReply, NavalBattlesRankingReply } from '../../generated/dashboard';
import { isBattleRow, isBattleValue, battleValueText } from '../battles/battleRows.ts';
import { isEngineHour } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
export { battleWindows as navalBattleWindows } from '../battles/battleRows.ts';

export const NAVAL_BATTLE_SIZE_LABELS = ['Ships sunk', 'Peak ships', 'Length'] as const;
export const NAVAL_BATTLE_DEFAULT_MEASURE = NAVAL_BATTLE_SIZE_LABELS[0];
export type NavalBattleMeasure = typeof NAVAL_BATTLE_SIZE_LABELS[number];
export type NavalBattleRow = BattleRow & {
  provinceId: number;
  windowIndex: number;
  observedHour: number;
  peakHour: number | null;
  measures: BattleValue[];
};
const span = (v: unknown): v is Record<string, unknown> & BattleWindow => object(v) && int(v.firstHour) && isEngineHour(v.firstHour)
  && int(v.lastHour) && isEngineHour(v.lastHour) && v.firstHour <= v.lastHour;
const warnings = (v: unknown) => Array.isArray(v) && v.every(w => object(w) && int(w.hour) && isEngineHour(w.hour)
  && typeof w.reason === 'string' && w.reason.length > 0 && int(w.count) && w.count > 0);
const contentWarnings = (v: unknown) => Array.isArray(v) && v.every((warning: unknown) => typeof warning === 'string');
function run(v: unknown): v is NavalBattleRun {
  if (!object(v) || !span(v) || !isBattleRow(v.row) || v.row.place.name !== null || v.row.sides.length !== 2
    || !(v.peakHour === null || int(v.peakHour) && isEngineHour(v.peakHour) && v.peakHour <= v.firstHour)
    || !Array.isArray(v.measures) || v.measures.length !== 3 || !v.measures.every(isBattleValue)
    || new Set(v.measures.map(value => value.label)).size !== 3) return false;
  const r = v as unknown as NavalBattleRun;
  return NAVAL_BATTLE_SIZE_LABELS.every(label => r.measures.some(value => value.label === label))
    && (r.measures.find(value => value.label === 'Peak ships')!.value === null ? r.peakHour === null : r.peakHour !== null)
    && (r.row.end === null || 'unknown' in r.row.end || r.row.end.hour <= r.firstHour)
    && (r.row.place.namedAt ?? r.row.place.hour) <= r.firstHour
    && r.row.sides.every(side => side.countries.every(country => country.hour === r.lastHour));
}
function history(v: unknown): v is NavalBattleHistory {
  if (!object(v) || !int(v.provinceId) || v.provinceId < 0 || v.provinceId > 0xffff_ffff
    || !Array.isArray(v.runs) || v.runs.length === 0 || !v.runs.every(run)) return false;
  const h = v as unknown as NavalBattleHistory;
  return h.runs.every((r, i) => r.row.id === h.id && (i === 0 || h.runs[i - 1].lastHour < r.firstHour));
}
const histories = (v: unknown): v is NavalBattleHistory[] => Array.isArray(v) && v.every(history)
  && new Set(v.map(h => h.id)).size === v.length;
export function validateNavalBattlesManifest(v: unknown): NavalBattlesManifestReply {
  const windows = object(v) && Array.isArray(v.windows) ? v.windows : null;
  if (!object(v) || windows === null || !windows.every(span)
    || !windows.every((w, i) => i === 0 || windows[i - 1].lastHour < w.firstHour)
    || typeof v.available !== 'boolean' || !(v.available ? v.unavailable === null : (v.unavailable === null && preparedPrefix(v)?.coverage.complete === false) || typeof v.unavailable === 'string' && v.unavailable.length > 0)
    || !contentWarnings(v.contentWarnings)) {
    throw new Error('The naval battles manifest is malformed.');
  }
  return v as unknown as NavalBattlesManifestReply;
}
export function validateNavalBattlesReply(v: unknown): NavalBattlesReply {
  if (!object(v) || !int(v.window) || v.window < 0 || !span(v.interval) || !histories(v.rows)
    || !Array.isArray(v.observed) || !v.observed.every(s => Array.isArray(s) && s.length === 2
      && int(s[0]) && isEngineHour(s[0]) && int(s[1]) && isEngineHour(s[1]) && s[0] <= s[1])
    || !warnings(v.warnings) || !contentWarnings(v.contentWarnings)) throw new Error('The naval battles reply is malformed.');
  const reply = v as unknown as NavalBattlesReply;
  if (reply.rows.some(h => h.runs.some(r => r.firstHour < reply.interval.firstHour || r.lastHour > reply.interval.lastHour))
    || reply.observed.some(([first, last]) => first < reply.interval.firstHour || last > reply.interval.lastHour)
    || reply.warnings.some(w => w.hour < reply.interval.firstHour || w.hour > reply.interval.lastHour)) {
    throw new Error('The naval battle window contains facts outside its interval.');
  }
  return reply;
}
export function validateNavalBattlesRanking(v: unknown): NavalBattlesRankingReply {
  if (!object(v) || !histories(v.rows) || !(v.censusHour === null || int(v.censusHour) && isEngineHour(v.censusHour))
    || !NAVAL_BATTLE_SIZE_LABELS.some(label => label === v.effectiveMeasure) || !warnings(v.warnings) || !contentWarnings(v.contentWarnings)) {
    throw new Error('The naval battles ranking is malformed.');
  }
  return v as unknown as NavalBattlesRankingReply;
}
export function navalBattleSize(row: NavalBattleRow, label: NavalBattleMeasure): BattleValue {
  return row.measures.find(measure => measure.label === label)!;
}
export function navalBattleMeasureText(value: BattleValue, row: NavalBattleRow): string {
  const text = `${battleValueText(value)}${value.value !== null && value.label === 'Length' ? ' h' : ''}`;
  if (value.value !== null && row.end === null && value.label === 'Ships sunk') return `${text} so far`;
  return text;
}

/** Merge loaded reads in recording order; prefetched future windows cannot backfill facts. */
type Windows = ReadonlyMap<number, NavalBattlesReply>;
type CountryMatch = (tag: string, hour: number) => boolean;
const rowPlaybacks = new WeakMap<Windows, WeakMap<CountryMatch, Map<string, PlaybackIntervals<ReadonlyArray<NavalBattleRow>>>>>();
const matchAll: CountryMatch = () => true;
const rankedWindows = new WeakMap<NavalBattlesRankingReply, Windows>();

export function navalBattleRows(windows: Windows, at: number, matches: CountryMatch, all: boolean, from: number | null = null): ReadonlyArray<NavalBattleRow> {
  let byMatch = rowPlaybacks.get(windows);
  if (!byMatch) rowPlaybacks.set(windows, byMatch = new WeakMap());
  let bySettings = byMatch.get(matches);
  if (!bySettings) byMatch.set(matches, bySettings = new Map());
  const key = JSON.stringify([all, from]);
  let playback = bySettings.get(key);
  if (!playback) {
    const matchingHours = countryMatchChangeHours(matches);
    function* changes() {
      yield* matchingHours ?? [];
      if (from !== null) yield from;
      for (const window of windows.values()) {
        yield window.interval.firstHour;
        for (const history of window.rows) {
          yield* spanChanges(history.runs);
        }
      }
    }
    playback = playbackRows(changes(), hour => readNavalBattleRows(windows, hour, matches, all, from), (rows, hour) =>
      !all && matchingHours === undefined || rows.some(row => row.observedHour === hour)
        ? { first: hour, end: hour + 1 } : { first: -Infinity, end: Infinity });
    bySettings.set(key, playback);
  }
  return playback.at(at);
}

function readNavalBattleRows(windows: ReadonlyMap<number, NavalBattlesReply>, at: number,
  matches: (tag: string, hour: number) => boolean, all: boolean, from: number | null = null): ReadonlyArray<NavalBattleRow> {
  const histories = new Map<string, { history: NavalBattleHistory; index: number }>();
  for (const [index, window] of [...windows].sort(([a], [b]) => a - b)) {
    if (window.interval.firstHour > at) continue;
    for (const h of window.rows) if (h.runs.some(run => run.firstHour <= at)) histories.set(h.id, { history: h, index });
  }
  const rows: NavalBattleRow[] = [];
  for (const { history: h, index } of histories.values()) {
    const run = h.runs.filter(run => run.firstHour <= at).at(-1);
    if (!run || run.row.end === null && run.lastHour < at) continue;
    const read = Math.min(at, run.lastHour);
    if (from !== null && (run.row.end && 'hour' in run.row.end ? run.row.end.hour < from : read < from)) continue;
    const sides: BattleSide[] = run.row.sides.map(side => ({ ...side,
      countries: side.countries.map(country => ({ ...country, hour: Math.min(read, country.hour) })) }));
    if (!all && !sides.some(side => side.countriesUnknown || side.countries.some(country => matches(country.tag, country.hour)))) continue;
    // Ended length is supplied by the native link, never extended to result_hour.
    const measures = run.measures.map(value => run.row.end === null && value.label === 'Length' && run.row.start !== null
      ? { label: 'Length', value: read - run.row.start.hour, atLeast: true } : value);
    rows.push({ ...run.row, provinceId: h.provinceId, windowIndex: index, sides, observedHour: read, peakHour: run.peakHour, measures });
  }
  return rows;
}
export function rankedNavalBattleRows(data: NavalBattlesRankingReply | undefined, at: number): ReadonlyArray<NavalBattleRow> {
  if (!data) return [];
  let windows = rankedWindows.get(data);
  if (!windows) {
    windows = new Map([[-1, { window: 0, interval: { firstHour: -Infinity, lastHour: Infinity }, rows: data.rows,
      observed: [], warnings: [], contentWarnings: data.contentWarnings }]]);
    rankedWindows.set(data, windows);
  }
  return navalBattleRows(windows, at, matchAll, true);
}
export { battleWarnings as navalBattleWarnings, shownBattleCensusHour as shownNavalCensusHour } from '../battles/battleRows.ts';
