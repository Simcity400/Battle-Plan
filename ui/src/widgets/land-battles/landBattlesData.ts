import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import { formatFixed } from '../../charts/valueAxis.ts';
import type { BattleWindow, LandBattleHistory, LandBattlesReply, LandBattlesManifestReply, LandBattlesRankingReply } from '../../generated/dashboard';
import type { BattleRow, BattleValue, BattleWarning } from '../battles/battleRows';
import { battleMeasure, battleValueText, isBattleValue } from '../battles/battleRows.ts';

/** A loss read as shown: each side's manpower and equipment total; the raw is that total's exact fixed5. */
export interface LandLossRead { hour: number; terminal: boolean; sides: [BattleValue[], BattleValue[]]; equipmentRaw?: [string | null, string | null] }
export interface LandPeak { value: number; hour: number }
/** `priorSidePeaks` is null for replies from builds that did not send each side's peak. */
export type LandBattle = Omit<LandBattleHistory, 'losses' | 'priorSidePeaks'> & { losses: LandLossRead[]; priorSidePeaks: [LandPeak | null, LandPeak | null] | null };
export type LandBattlesWindow = Omit<LandBattlesReply, 'rows' | 'reasons' | 'warnings'> & { rows: LandBattle[]; warnings: BattleWarning[] };
export type LandBattlesRanking = Omit<LandBattlesRankingReply, 'rows'> & { rows: LandBattle[] };

export interface LandBattleRow extends BattleRow {
  provinceId: number;
  windowIndex: number;
  readHour: number;
  peak: LandPeak;
  peakUnknown: boolean;
  /** Each side's most divisions at once, attacker then defender; null when not sent or no roster was read. */
  sidePeaks: [LandPeak | null, LandPeak | null] | null;
  length: BattleValue;
  casualties: BattleValue;
  lossRead: { hour: number; terminal: boolean; equipmentRaw?: [string | null, string | null] } | null;
}
const nullableText = (v: unknown) => v === null || typeof v === 'string';
const uint = (v: unknown): v is number => int(v) && v >= 0 && v <= 4294967295;
const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const moment = (v: unknown) => object(v) && hour(v.hour) && (v.bound === 'exact' || v.bound === 'by');
const peak = (v: unknown) => object(v) && int(v.value) && v.value >= 0 && hour(v.hour);
const span = (v: unknown) => object(v) && hour(v.firstHour) && hour(v.lastHour) && v.firstHour <= v.lastHour;
const warnings = (v: unknown) => Array.isArray(v) && v.every(w => object(w) && hour(w.hour) && typeof w.reason === 'string' && int(w.count) && w.count > 0);
const texts = (v: unknown): v is string[] => Array.isArray(v) && v.every((text: unknown) => typeof text === 'string');
const rawAmount = (v: unknown): v is string => typeof v === 'string' && v.length <= 40 && /^-?(0|[1-9]\d*)$/.test(v)
  && BigInt(v) >= -(1n << 127n) && BigInt(v) < (1n << 127n);
const MANPOWER = 'Manpower lost';
const EQUIPMENT = 'Equipment lost';
const MAX_EXACT = BigInt(Number.MAX_SAFE_INTEGER);
/** A safe fixed5 amount is a number, a wider one its exact text; the raw stays exact either way. */
function equipmentValue(label: string, raw: bigint): BattleValue {
  const amount = raw < 0n ? -raw : raw;
  return amount <= MAX_EXACT ? { label, value: Number(raw) / 100000 }
    : { label, value: null, unknown: `Exact amount: ${raw < 0n ? '-' : ''}${amount / 100000n}.${String(amount % 100000n).padStart(5, '0')}` };
}
type Variants = [number, string][];
const variants = (v: unknown, size: number): v is Variants => Array.isArray(v) && v.every(entry => Array.isArray(entry) && entry.length === 2
  && int(entry[0]) && entry[0] >= 0 && entry[0] < size && rawAmount(entry[1]));
interface LossSide { values: BattleValue[]; raw: string | null; variants: Variants | null }
/** A side's exact equipment total; replies from builds that listed each design
 * (by `labels`) are reduced to that total, by position against the previous read. */
function lossSide(v: unknown, labels: readonly string[] | null, previous: LossSide | null): LossSide | null {
  if (!object(v)) return null;
  const manpower: BattleValue = typeof v.manpower === 'number' ? { label: MANPOWER, value: v.manpower } : { label: MANPOWER, value: null, unknown: v.manpower as string };
  if (v.unknown !== undefined) return { values: [manpower, { label: EQUIPMENT, value: null, unknown: v.unknown as string }], raw: null, variants: null };
  if (v.equipment !== undefined) return rawAmount(v.equipment) && v.variants === undefined && v.changed === undefined
    ? { values: [manpower, equipmentValue(EQUIPMENT, BigInt(v.equipment))], raw: v.equipment, variants: null } : null;
  if (!labels) return null;
  let list: Variants;
  if (variants(v.variants, labels.length)) list = v.variants;
  else if (previous?.variants && variants(v.changed, previous.variants.length) && v.changed.every((entry, index, all) => index === 0 || entry[0] > all[index - 1][0])) {
    list = [...previous.variants];
    for (const [at, amount] of v.changed) list[at] = [list[at][0], amount];
  } else return null;
  if (new Set(list.map(([label]) => label)).size !== list.length) return null;
  const total = list.reduce((sum, [, amount]) => sum + BigInt(amount), 0n);
  return { values: [manpower, equipmentValue(EQUIPMENT, total)], raw: String(total), variants: list };
}
/** Rebuilds each read's full values; a side's changes apply to its previous read in the reply. */
function lossReads(v: unknown, start: unknown, labels: readonly string[] | null): LandLossRead[] | null {
  if (!Array.isArray(v) || !object(start) || !hour(start.hour)) return null;
  const reads: LandLossRead[] = [];
  let previous: (LossSide | null)[] = [null, null];
  for (const read of v) {
    const last = reads[reads.length - 1];
    if (!object(read) || !hour(read.hour) || read.hour < start.hour || typeof read.terminal !== 'boolean'
      || last && (read.hour < last.hour || last.terminal)) return null;
    const unread = read.sides;
    if (typeof unread === 'string') {
      const side = (): BattleValue[] => [{ label: MANPOWER, value: null, unknown: unread }, { label: EQUIPMENT, value: null, unknown: unread }];
      reads.push({ hour: read.hour, terminal: read.terminal, sides: [side(), side()] });
      previous = [null, null];
      continue;
    }
    if (!Array.isArray(read.sides) || read.sides.length !== 2) return null;
    const attacker = lossSide(read.sides[0], labels, previous[0]), defender = lossSide(read.sides[1], labels, previous[1]);
    if (!attacker || !defender) return null;
    previous = [attacker, defender];
    reads.push({ hour: read.hour, terminal: read.terminal, sides: [attacker.values, defender.values], equipmentRaw: [attacker.raw, defender.raw] });
  }
  return reads.every(read => read.sides.every(side => side.every(isBattleValue) && new Set(side.map(value => value.label)).size === side.length)) ? reads : null;
}
const sidePeaks = (v: unknown) => Array.isArray(v) && v.length === 2 && v.every(side => side === null || peak(side));
function battles(v: unknown, labels: unknown): LandBattle[] | null {
  if (!histories(v) || !(labels === undefined || texts(labels))) return null;
  const rows: LandBattle[] = [];
  for (const row of v as (Omit<LandBattleHistory, 'priorSidePeaks'> & { priorSidePeaks?: unknown })[]) {
    const losses = lossReads(row.losses, row.start, labels ?? null);
    if (!losses || !(row.priorSidePeaks == null || sidePeaks(row.priorSidePeaks))) return null;
    rows.push({ ...row, losses, priorSidePeaks: (row.priorSidePeaks as LandBattle['priorSidePeaks'] | undefined) ?? null });
  }
  return rows;
}
const histories = (v: unknown) => Array.isArray(v) && v.every(r => object(r) && typeof r.id === 'string' && uint(r.provinceId) && moment(r.start)
  && (r.end === null || moment(r.end) || object(r.end) && typeof r.end.unknown === 'string')
  && (r.endUnknownHour === null || hour(r.endUnknownHour)) && int(r.counterBase)
  && (r.priorPeak === null || peak(r.priorPeak)) && Array.isArray(r.runs) && r.runs.length > 0
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
    || !texts(v.contentWarnings))
    throw new Error('The land battles manifest is malformed.');
  return v as unknown as LandBattlesManifestReply;
}
/** Decodes the window's loss reads and interned warning reasons into full reads and warnings. */
export function validateLandBattlesReply(v: unknown): LandBattlesWindow {
  const rows = object(v) ? battles(v.rows, v.labels) : null;
  const reasons = object(v) && texts(v.reasons) ? v.reasons : null;
  if (!object(v) || !rows || !reasons || !span(v.interval) || !int(v.window) || v.window < 0
    || !Array.isArray(v.observed) || !v.observed.every(s => Array.isArray(s) && s.length === 2 && hour(s[0]) && hour(s[1]) && s[0] <= s[1])
    || !Array.isArray(v.warnings) || !v.warnings.every(w => Array.isArray(w) && w.length === 3 && hour(w[0])
      && int(w[1]) && w[1] >= 0 && w[1] < reasons.length && int(w[2]) && w[2] > 0)
    || !texts(v.contentWarnings)) throw new Error('The land battles reply is malformed.');
  return { window: v.window, interval: v.interval as BattleWindow, rows, observed: v.observed,
    warnings: v.warnings.map(([at, reason, count]) => ({ hour: at, reason: reasons[reason], count })), contentWarnings: v.contentWarnings };
}
export function validateLandBattlesRanking(v: unknown): LandBattlesRanking {
  const rows = object(v) ? battles(v.rows, v.labels) : null;
  if (!object(v) || !rows || !(v.censusHour === null || hour(v.censusHour))
    || !['Casualties', 'Peak size', 'Length'].includes(String(v.effectiveMeasure)) || !warnings(v.warnings) || !texts(v.contentWarnings))
    throw new Error('The land battles ranking is malformed.');
  return { rows, censusHour: v.censusHour as number | null, effectiveMeasure: v.effectiveMeasure as string,
    warnings: v.warnings as BattleWarning[], contentWarnings: v.contentWarnings };
}

/** A result side supplies the participants unless the result does not record that side while
 * the battle's last hourly read did; that read's countries stay, at their own date. */
const resultSide = (result: NonNullable<LandBattle['result']>, run: LandBattle['runs'][number], index: number) =>
  !result.countriesUnknown[index] || run.countriesUnknown[index];
/** Merge in recording order, independent of network completion order. */
export function landBattleRows(windows: ReadonlyMap<number, LandBattlesWindow>, at: number,
  matches: (tag: string, hour: number) => boolean, all: boolean, from: number | null = null): LandBattleRow[] {
  const histories = new Map<string, { history: LandBattle; index: number }>();
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
    const participants = [0, 1].map(index => result && resultSide(result, run, index)
      ? { tags: result.countries[index], hour: result.hour, unknown: result.countriesUnknown[index] }
      : { tags: run.countries[index], hour: read, unknown: run.countriesUnknown[index] });
    const countries = participants.map(side => side.tags.map(tag => ({ tag, hour: side.hour })));
    const countriesUnknown = participants.map(side => side.unknown);
    if (!all && !countriesUnknown.some(Boolean) && !countries.some(side => side.some(country => matches(country.tag, country.hour)))) continue;
    let end: BattleRow['end'] = h.end && ('hour' in h.end ? h.end.hour <= at : h.endUnknownHour !== null && h.endUnknownHour <= at) ? h.end : null;
    const endedAt = h.end && 'hour' in h.end ? h.end.hour : h.endUnknownHour;
    if (from !== null && read < from && !(endedAt !== null && endedAt >= from && endedAt <= at)) continue;
    if (end === null && read < at) end = { unknown: 'Ending not recorded' };
    let peak = h.priorPeak ?? { value: 0, hour: h.start.hour };
    let peakUnknown = h.priorPeak === null;
    const sides: [LandPeak | null, LandPeak | null] | null = h.priorSidePeaks && [h.priorSidePeaks[0], h.priorSidePeaks[1]];
    for (const run of runs) {
      if (run.rosterUnknown) continue;
      peakUnknown = false;
      const value = run.counts[0] + run.counts[1];
      if (value > peak.value || value === peak.value && run.firstHour < peak.hour) peak = { value, hour: run.firstHour };
      if (sides) for (const side of [0, 1]) {
        const best = sides[side];
        if (!best || run.counts[side] > best.value) sides[side] = { value: run.counts[side], hour: run.firstHour };
      }
    }
    const duration = result?.duration ?? run.duration + read - run.firstHour;
    let loss: LandLossRead | undefined;
    for (let index = h.losses.length - 1; index >= 0; index--) {
      if (h.losses[index].hour <= at) { loss = h.losses[index]; break; }
    }
    const row: LandBattleRow = { id: h.id, provinceId: h.provinceId, windowIndex: index, readHour: read, start: h.start, end, place: { name: null, hour: h.start.hour }, peak, peakUnknown,
      sidePeaks: sides,
      lossRead: loss ? { hour: loss.hour, terminal: loss.terminal, equipmentRaw: loss.equipmentRaw } : null,
      // Only a battle under way when its recorded stretch began has a length that is a minimum.
      length: { label: 'Length', value: Math.max((result?.hour ?? read) - h.start.hour, duration - h.counterBase, 0), atLeast: h.start.bound === 'by' },
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
  const status = row.lossRead.terminal ? (tooltip ? ' final' : '') : ' so far';
  return `${text}${status}${tooltip ? `, as of ${formatDateTime(row.lossRead.hour)}` : ''}`;
}
export function landDivisionText(value: BattleValue, compact: boolean): string {
  if (value.value === null) return compact ? '—' : value.unknown;
  const text = battleValueText(value);
  return compact ? text : `${text} ${value.value === 1 ? 'division' : 'divisions'}`;
}
export const landCasualtiesMissing = (rows: readonly LandBattleRow[]) => rows.length > 0 && rows.every(row => row.casualties.value === null);
/** Equipment shows whole numbers, as the game counts equipment, from the read's exact fixed5 total. */
export function landLossText(value: BattleValue, row: LandBattleRow, side: number, tooltip: boolean): string {
  const raw = value.label === EQUIPMENT ? row.lossRead?.equipmentRaw?.[side] : null;
  return raw == null ? landCasualtyText(value, row, tooltip) : datedLossText(formatFixed(raw, 5, 0), row, tooltip);
}
export function casualtyRankingNote(data: LandBattlesRanking | undefined): string | null {
  if (data?.effectiveMeasure === 'Peak size') return 'Casualties unavailable; ranked by peak size.';
  return null;
}
/** Ranking is computed before top N by the product, including country selection. */
export function rankedLandBattleRows(data: LandBattlesRanking | undefined, at: number): LandBattleRow[] {
  if (!data) return [];
  return landBattleRows(new Map([[-1, { window: 0, interval: { firstHour: at, lastHour: at }, rows: data.rows, observed: [], warnings: [], contentWarnings: data.contentWarnings }]]), at, () => true, true);
}
export { battleWindowIndex, battleWindows, battleWarnings, shownBattleCensusHour as shownCensusHour,
  battlePlaceRequests as landBattlePlaceRequests, namedBattles as namedLandBattles } from '../battles/battleRows.ts';
