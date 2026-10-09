import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import { formatFixed } from '../../charts/valueAxis.ts';
import type { LandBattleHistory, LandBattleLossRead, LandBattlesReply, LandBattlesManifestReply, LandBattlesRankingReply } from '../../generated/dashboard';
import type { BattleRow, BattleValue } from '../battles/battleRows';
import { battleMeasure, battleValueText, isBattleValue } from '../battles/battleRows.ts';

export interface LandBattleRow extends BattleRow {
  provinceId: number;
  windowIndex: number;
  readHour: number;
  peak: { value: number; hour: number };
  peakUnknown: boolean;
  length: BattleValue;
  casualties: BattleValue;
  lossRead: { hour: number; terminal: boolean; equipmentRaw?: [string[], string[]] } | null;
}
const nullableText = (v: unknown) => v === null || typeof v === 'string';
const uint = (v: unknown): v is number => int(v) && v >= 0 && v <= 4294967295;
const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const moment = (v: unknown) => object(v) && hour(v.hour) && (v.bound === 'exact' || v.bound === 'by');
const peak = (v: unknown) => object(v) && int(v.value) && v.value >= 0 && hour(v.hour);
const span = (v: unknown) => object(v) && hour(v.firstHour) && hour(v.lastHour) && v.firstHour <= v.lastHour;
const warnings = (v: unknown) => Array.isArray(v) && v.every(w => object(w) && hour(w.hour) && typeof w.reason === 'string' && int(w.count) && w.count > 0);
const contentWarnings = (v: unknown) => Array.isArray(v) && v.every((warning: unknown) => typeof warning === 'string');
const rawAmount = (v: unknown): v is string => typeof v === 'string' && v.length <= 40 && /^-?(0|[1-9]\d*)$/.test(v)
  && BigInt(v) >= -(1n << 127n) && BigInt(v) < (1n << 127n);
function lossReads(v: unknown, start: unknown): boolean {
  if (!Array.isArray(v) || !object(start) || !hour(start.hour)) return false;
  const first = start.hour;
  return v.every((read, index) => {
    if (!object(read) || !hour(read.hour) || read.hour < first || typeof read.terminal !== 'boolean') return false;
    const sides = read.sides;
    if (!Array.isArray(sides) || sides.length !== 2
      || !sides.every(side => Array.isArray(side) && side.every(isBattleValue) && new Set(side.map(value => value.label)).size === side.length)) return false;
    return (read.equipmentRaw === undefined || Array.isArray(read.equipmentRaw) && read.equipmentRaw.length === 2
      && read.equipmentRaw.every((side, sideIndex) => Array.isArray(side) && side.every(rawAmount)
        && (side.length === 0 || side.length === sides[sideIndex].length - 1)))
      && (index === 0 || read.hour >= v[index - 1].hour && !v[index - 1].terminal);
  });
}
const histories = (v: unknown) => Array.isArray(v) && v.every(r => object(r) && typeof r.id === 'string' && uint(r.provinceId) && moment(r.start)
  && (r.end === null || moment(r.end) || object(r.end) && typeof r.end.unknown === 'string')
  && (r.endUnknownHour === null || hour(r.endUnknownHour)) && int(r.counterBase)
  && (r.priorPeak === null || peak(r.priorPeak)) && Array.isArray(r.runs) && r.runs.length > 0
  && lossReads(r.losses, r.start)
  && (r.result === null || object(r.result) && hour(r.result.hour)
    && int(r.result.duration) && r.result.duration >= -2147483648 && r.result.duration <= 2147483647
    && object(r.result.winner) && (r.result.winner.side === 0 || r.result.winner.side === 1 || typeof r.result.winner.unknown === 'string')
    && Array.isArray(r.result.countries) && r.result.countries.length === 2 && r.result.countries.every(tags => Array.isArray(tags) && tags.every(tag => typeof tag === 'string'))
    && Array.isArray(r.result.countriesUnknown) && r.result.countriesUnknown.length === 2 && r.result.countriesUnknown.every(value => typeof value === 'boolean'))
  && r.runs.every(run => span(run) && object(run) && Array.isArray(run.counts) && run.counts.length === 2 && run.counts.every(uint)
    && typeof run.rosterUnknown === 'boolean'
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
    const result = h.result && h.result.hour <= at ? h.result : null;
    const countries = (result?.countries ?? run.countries).map(tags => tags.map(tag => ({ tag, hour: result?.hour ?? read })));
    const countriesUnknown = result?.countriesUnknown ?? run.countriesUnknown;
    if (!all && !countriesUnknown.some(Boolean) && !countries.some(side => side.some(country => matches(country.tag, country.hour)))) continue;
    let end: BattleRow['end'] = h.end && ('hour' in h.end ? h.end.hour <= at : h.endUnknownHour !== null && h.endUnknownHour <= at) ? h.end : null;
    const endedAt = h.end && 'hour' in h.end ? h.end.hour : h.endUnknownHour;
    if (from !== null && read < from && !(endedAt !== null && endedAt >= from && endedAt <= at)) continue;
    if (end === null && read < at) end = { unknown: 'Ending not recorded' };
    let peak = h.priorPeak ?? { value: 0, hour: h.start.hour };
    let peakUnknown = h.priorPeak === null;
    for (const run of runs) {
      if (run.rosterUnknown) continue;
      peakUnknown = false;
      const value = run.counts[0] + run.counts[1];
      if (value > peak.value || value === peak.value && run.firstHour < peak.hour) peak = { value, hour: run.firstHour };
    }
    const duration = result?.duration ?? run.duration + read - run.firstHour;
    let loss: LandBattleLossRead | undefined;
    for (let index = h.losses.length - 1; index >= 0; index--) {
      if (h.losses[index].hour <= at) { loss = h.losses[index]; break; }
    }
    const row: LandBattleRow = { id: h.id, provinceId: h.provinceId, windowIndex: index, readHour: read, start: h.start, end, place: { name: null, hour: h.start.hour }, peak, peakUnknown,
      lossRead: loss ? { hour: loss.hour, terminal: loss.terminal, equipmentRaw: loss.equipmentRaw } : null,
      length: { label: 'Length', value: Math.max((result?.hour ?? read) - h.start.hour, duration - h.counterBase, 0), atLeast: true },
      casualties: { label: 'Casualties', value: null, unknown: 'Not in this recording' },
      sides: ['Attackers', 'Defenders'].map((label, index) => ({ label, countries: countries[index],
        ...(countriesUnknown[index] ? { countriesUnknown: 'Countries not recorded' } : {}),
        counts: [run.rosterUnknown ? { label: 'Divisions (incl. reserves)', value: null, unknown: 'Roster not recorded' }
          : { label: 'Divisions (incl. reserves)', value: run.counts[index] }], losses: loss?.sides[index] ?? [] })),
      winner: result?.winner ?? { unknown: 'Result not recorded' } };
    row.casualties = { ...battleMeasure(row, 'Manpower lost', 'Not in this recording'), label: 'Casualties' };
    rows.push(row);
  }
  return rows;
}
export function landCasualtyText(value: BattleValue, row: LandBattleRow, tooltip = false): string {
  const text = battleValueText(value);
  if (value.value === null || !row.lossRead) return text;
  return datedLossText(text, row, tooltip);
}
function datedLossText(text: string, row: LandBattleRow, tooltip: boolean): string {
  if (!row.lossRead) return text;
  const status = row.lossRead.terminal ? (tooltip ? ' terminal' : '') : ' so far';
  return `${text}${status}${tooltip ? `, as of ${formatDateTime(row.lossRead.hour)}` : ''}`;
}
export function landDivisionText(value: BattleValue, compact: boolean): string {
  if (value.value === null) return compact ? '—' : value.unknown;
  const text = battleValueText(value);
  return compact ? text : `${text} ${value.value === 1 ? 'division' : 'divisions'}`;
}
export const landCasualtiesMissing = (rows: readonly LandBattleRow[]) => rows.length > 0 && rows.every(row => row.casualties.value === null);
export function landLossText(value: BattleValue, row: LandBattleRow, side: number, tooltip: boolean): string {
  const index = row.sides[side].losses.findIndex(loss => loss.label === value.label) - 1;
  const raw = index >= 0 ? row.lossRead?.equipmentRaw?.[side]?.[index] : undefined;
  return raw === undefined ? landCasualtyText(value, row, tooltip) : datedLossText(formatFixed(raw, 5, 5), row, tooltip);
}
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
export { battleWindowIndex, battleWindows, battleWarnings, shownBattleCensusHour as shownCensusHour,
  battlePlaceRequests as landBattlePlaceRequests, namedBattles as namedLandBattles } from '../battles/battleRows.ts';
