/** The Equipment produced widget's data: the `equipment-produced` reply checked and read. A reply is one window of
 * hours (`crates/dashboard/src/products/equipment_produced.rs`); the windows held join into one model, so the race at
 * any hour they hold, the playhead's included, and the lines are lookups in it, never a request. An unknown value
 * stays unknown here: it is never drawn, ranked or summed as zero. */
import type { CountryRef } from '../../generated/dashboard';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { Run, Span } from '../../charts/series';
import { shownRuns } from '../../charts/race/lines.ts';
import { runIndexAt } from '../../charts/series.ts';
import { formatAmount, formatRate } from '../../charts/valueAxis.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { readAt } from '../../dashboard/reads.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import type { CountryIdentityRun, EquipmentProducedReply, ProducedCount, ProducedMeasure, ProducedSeries, ProducedUnknown } from '../../generated/dashboard';
import { WINDOW_HOURS, windowsOf } from '../../dashboard/heldWindows.ts';

/** The reply's data over the windows joined, which hold no one window's hours. */
export type ProducedData = Omit<EquipmentProducedReply, 'window' | 'firstHour' | 'lastHour'>;

export type Measure = ProducedMeasure;
export type Count = ProducedCount;

/** The windows the widget offers, per day first. The product also counts a week. */
export const MEASURES: readonly { value: Measure; label: string }[] = [
  { value: 'day', label: 'Per day' },
  { value: 'month', label: 'Per month' },
  { value: 'total', label: 'Since recording start' },
];
const REPLY_MEASURES: readonly Measure[] = ['total', 'day', 'week', 'month'];

/** Why a value is not known, in words. */
export const UNKNOWN_WORDS: Record<ProducedUnknown, string> = {
  notRecorded: 'Production was not recorded for this country then.',
  unitsInexact: 'A production line left a delivery out of its count.',
  spendingUnavailable: 'The factory IC a line applied was not recorded.',
  lostOutput: 'A production line went without its final count.',
  missedPublication: 'Production was not read at some hours.',
  typeNotRecorded: 'Designs whose type was not recorded hold output that could be of this type.',
  windowIncomplete: 'The window reaches before this country\'s recorded production.',
  observationRestarted: 'Production counting began again inside the window.',
  rocketTableUnavailable: 'Rocket site output was not recorded then.',
  rocketInputInexact: 'Rocket site output was not counted exactly.',
  rocketOwnerUnreadable: 'Rocket site output of an unknown country was recorded.',
  outOfRange: 'The amount is too large to show exactly.',
};
const REASONS = new Set(Object.keys(UNKNOWN_WORDS));

const INT = (value: unknown): value is number => Number.isInteger(value);
const ascending = (values: unknown): values is number[] => Array.isArray(values) && values.every((value, index) => INT(value) && (index === 0 || values[index - 1] < value));

/** A series' runs in hour order without overlaps, its gaps and breaks pointing at runs that exist. */
export function validSeries(series: ProducedSeries): boolean {
  const count = Array.isArray(series?.firstHours) ? series.firstHours.length : -1;
  return count >= 0 && Array.isArray(series.lastHours) && series.lastHours.length === count && Array.isArray(series.values) && series.values.length === count
    && series.firstHours.every((first, index) => INT(first) && INT(series.lastHours[index]) && first <= series.lastHours[index]
      && (index === 0 || series.lastHours[index - 1] < first))
    && series.values.every((value, index) => value === null ? series.unknown.some(gap => gap.run === index) : Number.isSafeInteger(value))
    && Array.isArray(series.unknown) && series.unknown.every((gap, index) => INT(gap.run) && gap.run >= 0 && gap.run < count && REASONS.has(gap.reason)
      && series.values[gap.run] === null && (index === 0 || series.unknown[index - 1].run < gap.run))
    && ascending(series.breaks) && series.breaks.every(index => index >= 0 && index < count);
}

/** A reply whose lines are keyed as the widget keys them: each `tag` its country's key (`countryKey`). */
export function validateProducedReply(data: unknown): ProducedData {
  const asked = data as ProducedData;
  const reply = asked && typeof asked === 'object' && Array.isArray(asked.lines)
    && asked.lines.some(line => line && (typeof line.country === 'string' || typeof line.country === 'number') && line.tag !== countryKey(line.country))
    ? { ...asked, lines: asked.lines.map(line => line && (typeof line.country === 'string' || typeof line.country === 'number') ? { ...line, tag: countryKey(line.country) } : line) }
    : asked;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && REPLY_MEASURES.includes(reply.measure)
    && (reply.count === 'units' || reply.count === 'ic') && INT(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.types) && reply.types.every(type => typeof type === 'string')
    && (reply.rankHour === null || INT(reply.rankHour))
    && Array.isArray(reply.lines) && reply.lines.every(line => typeof line.tag === 'string' && validCountryIdentityRuns(line.identities) && validSeries(line.series))
    && new Set(reply.lines.map(line => line.tag)).size === reply.lines.length
    && ascending(reply.reads) && ascending(reply.observations)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The equipment produced reply is malformed.');
  return reply;
}

/** What the widget asks for: chosen countries, or the top N ranked once at `rankHour` (the view's last hour or the
 * cursor); never an hour that moves with the playhead. */
export interface ProducedRequest {
  branchId: string; countries: CountryRef[]; top?: number; rankHour?: number;
  count: Count; measure: Measure; types: string[]; refits: boolean;
}

export function producedRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, rankHour: number, count: Count,
  measure: Measure, types: readonly string[], refits: boolean): ProducedRequest {
  const common = { branchId, count, measure, types: [...types], refits };
  return resolved.top != null ? { ...common, countries: [], top: resolved.top, rankHour: Math.floor(rankHour) } : { ...common, countries: resolved.tags.map(countryFromKey) };
}

/** The hour a top N is ranked at: the pinned moment; following, the hour shown whenever paused or sought, kept while
 * playing (`held`) so playback asks nothing; else the view's last hour. Ranked at the branch's end, a race at any
 * earlier hour would show only the end's top N. */
export function producedRankHour(mode: string, playhead: number | null, playing: boolean, held: number | null, viewLast: number): number {
  if (mode === 'pin' && playhead != null) return playhead;
  if (mode === 'follow' && playhead != null) return playing && held != null ? held : playhead;
  return viewLast;
}

/** A value at an hour: its raw number (null where not known, with why), the run it is in and the unbroken stretch. */
export interface ValueAt { value: number | null; reason: ProducedUnknown | null; index: number; stretch: number }

/** A series ready for lookups. */
export interface SeriesModel {
  series: ProducedSeries;
  runs: readonly Span[];
  stretch: readonly number[];
  reasons: ReadonlyMap<number, ProducedUnknown>;
}

export function seriesModel(series: ProducedSeries): SeriesModel {
  const runs = series.firstHours.map((firstHour, index) => ({ firstHour, lastHour: series.lastHours[index] }));
  const breaks = new Set(series.breaks);
  const stretch: number[] = [];
  let at = 0;
  runs.forEach((run, index) => {
    if (index > 0 && (breaks.has(index) || runs[index - 1].lastHour + 1 !== run.firstHour)) at += 1;
    stretch.push(at);
  });
  return { series, runs, stretch, reasons: new Map(series.unknown.map(gap => [gap.run, gap.reason])) };
}

export function valueAt(model: SeriesModel, hour: number): ValueAt | null {
  const index = runIndexAt(model.runs, Math.floor(hour));
  if (index < 0) return null;
  const value = model.series.values[index];
  return { value, reason: value === null ? model.reasons.get(index) ?? 'notRecorded' : null, index, stretch: model.stretch[index] };
}

/** A reply ready for lookups, built once per reply. */
export interface ProducedModel {
  reply: ProducedData;
  lines: ReadonlyMap<string, SeriesModel & { identities: readonly CountryIdentityRun[] }>;
  tags: readonly string[];
}

export function producedModel(reply: ProducedData): ProducedModel {
  return { reply, lines: new Map(reply.lines.map(line => [line.tag, { ...seriesModel(line.series), identities: line.identities }])), tags: reply.lines.map(line => line.tag) };
}

export const producedAt = (model: ProducedModel, tag: string, hour: number): ValueAt | null => {
  const line = model.lines.get(tag);
  return line ? valueAt(line, hour) : null;
};

/** The countries ranked by their value at an hour, greatest first (ties by tag), at most `count`; a country not
 * recorded then, or whose value is unknown, is left out, never ranked as zero. With `produced` (a top N), a
 * country ranks only once it has produced something. */
export function rankProduced(model: ProducedModel, candidates: readonly string[], hour: number, count: number,
  produced = false): { tag: string; at: ValueAt & { value: number } }[] {
  return candidates.flatMap(tag => {
    const at = producedAt(model, tag, hour);
    return at && at.value !== null && !(produced && at.value === 0) ? [{ tag, at: { ...at, value: at.value } }] : [];
  }).sort((a, b) => b.at.value - a.at.value || compareTags(a.tag, b.tag)).slice(0, Math.max(0, count));
}

/** A recorded integer as the number the charts plot, in the count's own unit. */
export const plotted = (raw: number, decimals: number) => raw / 10 ** decimals;

/** A recorded integer written exactly: units whole; IC by the rate rule, so a small nonzero amount never reads 0. */
export const formatRaw = (raw: number, decimals: number) =>
  decimals === 0 ? formatAmount(BigInt(raw), 0, 0) : formatRate(BigInt(raw), decimals);

/** A plotted value written as `formatRaw` writes its recorded integer. */
export const formatProduced = (value: number, decimals: number) => formatRaw(Math.round(value * 10 ** decimals), decimals);

/** The race's entries at an hour. */
export function raceEntries(ranked: readonly { tag: string; at: ValueAt & { value: number } }[], decimals: number, hour: number): CountryRaceEntry[] {
  return ranked.map(({ tag, at }) => ({ tag, value: plotted(at.value, decimals), label: formatRaw(at.value, decimals), stretch: at.stretch, hour }));
}

/** A series' values as plotting runs: an unknown run leaves a gap and the next known one starts a new line. */
export function seriesRuns(model: SeriesModel, decimals: number): Run[] {
  const runs: Run[] = [];
  let broken = true;
  const breaks = new Set(model.series.breaks);
  model.runs.forEach((span, index) => {
    const raw = model.series.values[index];
    if (raw === null) { broken = true; return; }
    const value = plotted(raw, decimals);
    const previous = runs.at(-1);
    const breakBefore = broken || breaks.has(index) || !previous || previous.lastHour + 1 !== span.firstHour;
    if (previous && !breakBefore && previous.value === value) previous.lastHour = span.lastHour;
    else runs.push({ firstHour: span.firstHour, lastHour: span.lastHour, value, ...(breakBefore ? { breakBefore: true } : {}) });
    broken = false;
  });
  return runs;
}

/** A country's line, cut to the hours it is shown (a group's membership; null: whenever recorded) and the view. */
export function lineRuns(model: ProducedModel, tag: string, membership: readonly Span[] | null, range: Span): Run[] {
  const line = model.lines.get(tag);
  return line ? shownRuns(seriesRuns(line, model.reply.decimals), membership, range) : [];
}

/** Where the running totals began counting at an hour: the latest observation then. */
export const countedSince = (observations: readonly number[], hour: number): number | null => readAt(observations, hour);

/** A type's name in the settings list: content's, else its recorded key. */
export const typeLabel = (type: { key: string; name: string | null }) => contentTitle(type.name, type.key, 'Equipment type name unavailable');

/** The structure a race shows at an hour: which rows, whatever their order. */
export const raceRows = (entries: readonly CountryRaceEntry[]) => entries.map(entry => entry.tag).sort().join(',');

/** The windows holding `firstHour..=lastHour`, in order. */
export const producedWindows = (firstHour: number, lastHour: number) => windowsOf(firstHour, lastHour);

/** One window of the reply, checked: its hours inside the window it names, its runs inside its hours. */
export function validateProducedWindow(data: unknown): EquipmentProducedReply {
  const reply = validateProducedReply(data) as EquipmentProducedReply;
  const ok = INT(reply.window) && reply.window >= 0 && INT(reply.firstHour) && INT(reply.lastHour)
    && (reply.firstHour > reply.lastHour || (reply.firstHour >= reply.window * WINDOW_HOURS && reply.lastHour < (reply.window + 1) * WINDOW_HOURS))
    && reply.lines.every(line => line.series.firstHours.every((first, index) => first >= reply.firstHour && line.series.lastHours[index] <= reply.lastHour));
  if (!ok) throw new Error('The equipment produced window is malformed.');
  return reply;
}

/** `next`'s runs after `series`'s, a run continuing the one before with the same value joined to it unless it starts
 * afresh: the windows joined are the reply of their hours together. */
function joinSeries(series: ProducedSeries, next: ProducedSeries): void {
  const reasons = new Map(next.unknown.map(gap => [gap.run, gap.reason]));
  const breaks = new Set(next.breaks);
  next.firstHours.forEach((firstHour, index) => {
    const value = next.values[index];
    const reason = value === null ? reasons.get(index) ?? 'notRecorded' : null;
    const open = series.firstHours.length;
    const held = open > 0 && series.values[open - 1] === null ? series.unknown.at(-1)?.reason ?? null : null;
    if (!breaks.has(index) && open > 0 && series.lastHours[open - 1] + 1 === firstHour && series.values[open - 1] === value && held === reason) {
      series.lastHours[open - 1] = next.lastHours[index];
      return;
    }
    series.firstHours.push(firstHour);
    series.lastHours.push(next.lastHours[index]);
    series.values.push(value);
    if (reason) series.unknown.push({ run: open, reason });
    if (breaks.has(index)) series.breaks.push(open);
  });
}

/** The windows held, joined in hour order into one reply; null while none is held. */
export function mergeProducedWindows(windows: ReadonlyMap<number, EquipmentProducedReply>): ProducedData | null {
  const held = [...windows.values()].sort((a, b) => a.window - b.window);
  if (!held.length) return null;
  const lines = new Map<string, { identities: readonly CountryIdentityRun[]; series: ProducedSeries }>();
  const reads = new Set<number>(), observations = new Set<number>(), warnings = new Set<string>();
  for (const reply of held) {
    for (const line of reply.lines) {
      let joined = lines.get(line.tag);
      if (!joined) lines.set(line.tag, joined = { identities: line.identities, series: { firstHours: [], lastHours: [], values: [], unknown: [], breaks: [] } });
      joinSeries(joined.series, line.series);
    }
    for (const read of reply.reads) reads.add(read);
    for (const observation of reply.observations) observations.add(observation);
    for (const warning of reply.warnings) warnings.add(warning);
  }
  const { branchId, count, measure, decimals, types, rankHour } = held[0];
  return { branchId, count, measure, decimals, types, rankHour,
    lines: [...lines].map(([tag, line]) => ({ country: countryFromKey(tag), tag, identities: [...line.identities], series: line.series })),
    reads: [...reads].sort((a, b) => a - b), observations: [...observations].sort((a, b) => a - b), warnings: [...warnings] };
}
