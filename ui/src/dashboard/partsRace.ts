/** Recorded totals and parallel content parts for country races, lines and mix. */
import type { CountryRaceEntry } from '../charts/race/countryRace';
import type { Run, Span } from '../charts/series';
import { isOrderedSpans, runAt, runIndexAt, spansCovered, stretchStarts } from '../charts/series.ts';
import { shownRuns } from '../charts/race/lines.ts';
import type { TooltipContent, TooltipRow } from '../charts/tooltip';
import { compareTags } from './tagOrder.ts';
import { object, int } from './validation.ts';

export interface PartsRaceRun extends Span {
  startsSegment: boolean; total: number | null; values: (number | null)[];
  referenceTotal?: number | null;
}
export interface PartsRaceSeries {
  tag: string; runs: PartsRaceRun[]; segments: Span[]; membership: Span[] | null;
}
export interface PartsRaceReply {
  branchId: string;
  parts: { key: string; name: string | null; unresolved: boolean }[];
  series: PartsRaceSeries[]; warnings: string[];
}
/** Display names/refusals may refresh without changing any recorded operand. */
export function partsRaceNumbers(reply: PartsRaceReply): PartsRaceReply {
  return { ...reply, warnings: [], parts: reply.parts.map(part => ({ key: part.key, name: null, unresolved: false })) };
}

const count = (value: unknown) => value === null || (int(value) && value >= 0);

/** Refuse malformed counts, catalogues and spans before any playback lookup. */
export function validatePartsRaceReply(data: unknown): PartsRaceReply {
  const fail = () => { throw new Error('The parts race reply is malformed.'); };
  if (!object(data) || typeof data.branchId !== 'string' || !Array.isArray(data.parts)
    || !Array.isArray(data.series) || !Array.isArray(data.warnings) || !data.warnings.every(item => typeof item === 'string')) return fail();
  const keys = new Set<string>();
  for (const part of data.parts) {
    if (!object(part) || typeof part.key !== 'string' || !part.key.length || keys.has(part.key)
      || (part.name !== null && typeof part.name !== 'string') || typeof part.unresolved !== 'boolean') return fail();
    keys.add(part.key);
  }
  const tags = new Set<string>();
  for (const series of data.series) {
    if (!object(series) || typeof series.tag !== 'string' || !series.tag.length || tags.has(series.tag)
      || !isOrderedSpans(series.runs) || !isOrderedSpans(series.segments)
      || !(series.membership === null || isOrderedSpans(series.membership))
      || !spansCovered(series.runs, series.segments)) return fail();
    tags.add(series.tag);
    for (const run of series.runs) {
      if (typeof run.startsSegment !== 'boolean' || !count(run.total) || !Array.isArray(run.values)
        || run.values.length !== keys.size || !run.values.every(count)
        || (run.referenceTotal !== undefined && !count(run.referenceTotal))) return fail();
      const knownTotal = run.values.reduce<number>((sum, value) => sum + (value ?? 0), 0);
      if (!Number.isSafeInteger(knownTotal) || (typeof run.total === 'number' && (knownTotal > run.total
        || (run.values.every(value => value !== null) && knownTotal !== run.total)))) return fail();
    }
  }
  return data as unknown as PartsRaceReply;
}

interface PartsRaceSeriesModel { series: PartsRaceSeries; starts: readonly number[] }
export interface PartsRaceModel { reply: PartsRaceReply; series: ReadonlyMap<string, PartsRaceSeriesModel>; tags: readonly string[] }
export function partsRaceModel(reply: PartsRaceReply): PartsRaceModel {
  return { reply, series: new Map(reply.series.map(series => [series.tag, { series,
    starts: stretchStarts(series.runs.map((run, index) => ({ ...run,
      breakBefore: run.total === null || series.runs[index - 1]?.total === null }))) }])),
    tags: reply.series.map(series => series.tag) };
}

export function hasPartsRaceValues(model: PartsRaceModel): boolean {
  return model.reply.series.some(series => series.runs.some(run => run.total !== null));
}

export interface PartsRaceAt { total: number | null; values: readonly (number | null)[]; run: PartsRaceRun; stretch: number }
export function partsRaceAt(model: PartsRaceModel, tag: string, hour: number): PartsRaceAt | null {
  const item = model.series.get(tag);
  if (!item) return null;
  const index = runIndexAt(item.series.runs, Math.floor(hour));
  if (index < 0) return null;
  const run = item.series.runs[index];
  return { total: run.total, values: run.values, run, stretch: item.starts[index] };
}

/** Looks use full identity spans, independent of a country's top-N membership. */
export function partsRaceSegmentAt(model: PartsRaceModel | null, tag: string, hour: number): number | null {
  return runAt(model?.series.get(tag)?.series.segments ?? [], Math.floor(hour))?.firstHour ?? null;
}

export function rankPartsRace(model: PartsRaceModel, candidates: readonly string[], hour: number, limit: number): { tag: string; at: PartsRaceAt & { total: number } }[] {
  return candidates.flatMap(tag => {
    const at = partsRaceAt(model, tag, hour);
    return at?.total != null ? [{ tag, at: { ...at, total: at.total } }] : [];
  }).sort((a, b) => b.at.total - a.at.total || compareTags(a.tag, b.tag)).slice(0, Math.max(0, limit));
}

export const formatPartsRaceValue = (value: number) => value.toLocaleString('en-US');
export const partsRacePartName = (part: PartsRaceReply['parts'][number]) => part.name || part.key;

/** A missing classification keeps the independently recorded total visible. */
export function partsRaceEntries(model: PartsRaceModel, ranked: ReturnType<typeof rankPartsRace>, split: boolean,
  formatValue = formatPartsRaceValue): CountryRaceEntry[] {
  return ranked.map(({ tag, at }) => {
    const known = at.values.reduce<number>((sum, value) => sum + (value ?? 0), 0);
    // A type read cannot make a country total less certain or discard its remainder.
    const complete = at.values.every(value => value != null) && known === at.total;
    return { tag, value: at.total, label: formatValue(at.total), stretch: at.stretch,
      ...(split && complete ? { parts: at.values.map((value, index) => ({ id: model.reply.parts[index].key, value: value! })) }
        : split ? { hatched: at.total } : {}) };
  });
}

/** Nullable values are gaps; equal neighbours merge only within one identity. */
export function partsRaceRuns(model: PartsRaceModel, tag: string, part?: number): Run[] {
  const result: Run[] = [];
  for (const run of model.series.get(tag)?.series.runs ?? []) {
    const value = part === undefined ? run.total : run.values[part];
    if (value == null) continue;
    const previous = result.at(-1);
    if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.value === value) previous.lastHour = run.lastHour;
    else result.push({ firstHour: run.firstHour, lastHour: run.lastHour, value, ...(run.startsSegment ? { breakBefore: true } : {}) });
  }
  return result;
}

export function partsRaceLineRuns(model: PartsRaceModel, tag: string, membership: readonly Span[] | null, range: Span): Run[] {
  return shownRuns(partsRaceRuns(model, tag), membership, range);
}
export function partsRaceMixBands(model: PartsRaceModel, tag: string, membership: readonly Span[] | null, range: Span): Run[][] {
  return model.reply.parts.map((_, index) => shownRuns(partsRaceRuns(model, tag, index), membership, range));
}

export interface PartsRaceTooltipOptions {
  model: PartsRaceModel; name: string; at: PartsRaceAt; date: string; measure: string; ownership: string;
  colours: ReadonlyMap<string, string>; note?: string; formatValue?: (value: number) => string; referenceLabel?: string;
}

/** Full and compact forms both preserve unknown parts and the independent total. */
export function* partsRaceTooltips({ model, name, at, date, measure, ownership, colours, note,
  formatValue = formatPartsRaceValue, referenceLabel }: PartsRaceTooltipOptions): Generator<TooltipContent> {
  const rows: TooltipRow[] = model.reply.parts.map((part, index) => ({ label: partsRacePartName(part),
    value: at.values[index] == null ? 'Not known' : formatValue(at.values[index]!),
    ...(colours.get(part.key) ? { swatch: colours.get(part.key) } : {}),
    ...(at.values[index] == null ? { tone: 'muted' as const } : {}) }));
  const total: TooltipRow = { label: measure, value: at.total == null ? 'Not known' : formatValue(at.total), strong: true, divider: true };
  const totals: TooltipRow[] = [total, ...(referenceLabel ? [{ label: referenceLabel,
    value: at.run.referenceTotal == null ? 'Not known' : formatValue(at.run.referenceTotal) }] : [])];
  const base = { title: name, subtitle: `${date} · ${ownership}`, ...(note ? { note } : {}) };
  yield { ...base, rows: [...rows, ...totals] };
  yield { ...base, rows: [...rows.filter((_, index) => at.values[index] !== 0), ...totals] };
  yield { ...base, rows: totals };
}
