import type { CountryIdentityRun } from '../../generated/dashboard';
import { shownRuns } from './lines.ts';
import type { Run, Span } from '../series';
import { isOrderedSpans, runAt, runIndexAt, spansCovered, stretchStarts } from '../series.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { int, object } from '../../dashboard/validation.ts';

export interface CountryValueType { key: string; name: string | null }
interface CountryValueRun extends Span {
  startsSegment: boolean; total: number | null; rawTotal?: string; values: (number | null)[]; rawValues?: (string | null)[];
}
export type CompactCountryValueRun = [number, number | null, number | null, string?];
export interface CountryValueMixRun extends Span { values: (number | null)[]; rawValues?: (string | null)[] }
export interface CountryValueSeries {
  tag: string; identities?: CountryIdentityRun[]; runs: CompactCountryValueRun[]; segments: Span[]; membership: Span[] | null;
  mix?: CountryValueMixRun[];
}
export interface CountryValuesReply { types: CountryValueType[]; series: CountryValueSeries[] }

export interface CountryPartsReply {
  branchId: string;
  parts: (CountryValueType & { unresolved: boolean })[];
  series: {
    tag: string; runs: CountryValueRun[];
    segments: Span[]; membership: Span[] | null;
  }[];
  warnings: string[];
}

const amount = (value: unknown) => value === null || (typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= Number.MAX_SAFE_INTEGER);

export function validCountryValues(reply: unknown): reply is CountryValuesReply {
  if (!object(reply) || !Array.isArray(reply.types) || !Array.isArray(reply.series)) return false;
  const types = reply.types;
  return types.every(type => object(type) && typeof type.key === 'string' && type.key.length > 0 && (type.name === null || typeof type.name === 'string'))
    && new Set(types.map(type => type.key)).size === types.length
    && reply.series.every(series => {
      if (!object(series) || typeof series.tag !== 'string' || !isOrderedSpans(series.segments)
        || !(series.membership === null || isOrderedSpans(series.membership)) || !Array.isArray(series.runs)) return false;
      if (!series.runs.every(run => Array.isArray(run) && (run.length === 3 || run.length === 4) && int(run[0])
        && (run.length === 3 ? amount(run[1]) : typeof run[3] === 'string' && /^-?(0|[1-9][0-9]*)$/.test(run[3]) && typeof run[1] === 'number' && Number.isFinite(run[1])) && (run[2] === null || (int(run[2]) && run[2] >= run[0])))) return false;
      const runs = normalRuns(series as unknown as CountryValueSeries);
      if (!isOrderedSpans(runs) || !spansCovered(runs, series.segments)
        || (series.membership !== null && !spansCovered(runs, series.membership))) return false;
      return series.mix === undefined || (Array.isArray(series.mix) && isOrderedSpans(series.mix)
        && spansCovered(series.mix, runs) && series.mix.every(({ values, rawValues }) => Array.isArray(values)
          && values.length === types.length && (rawValues === undefined ? values.every(amount)
            : Array.isArray(rawValues) && rawValues.length === types.length
              && rawValues.every((raw, index) => raw === null ? values[index] === null
                : typeof raw === 'string' && /^(0|[1-9][0-9]*)$/.test(raw)
                  && typeof values[index] === 'number' && Number.isFinite(values[index])
                  && Number(raw) === values[index]))));
    })
    && new Set(reply.series.map(series => series.tag)).size === reply.series.length;
}

function normalRuns(series: CountryValueSeries): CountryValueRun[] {
  return series.runs.map(run => ({
    firstHour: run[0], lastHour: run[2] ?? run[0], total: run[1], rawTotal: run[3], values: [],
    startsSegment: series.segments.some(segment => segment.firstHour === run[0]),
  }));
}

export interface CountryValuesModel {
  reply: CountryValuesReply;
  series: ReadonlyMap<string, Omit<CountryValueSeries, 'runs'> & { runs: CountryValueRun[]; parts: CountryValueRun[]; starts: readonly number[]; unknown: readonly null[] }>;
  tags: readonly string[];
}

export function countryValuesModel(reply: CountryValuesReply): CountryValuesModel {
  return { reply, series: new Map(reply.series.map(series => {
    const totals = normalRuns(series);
    const parts = series.mix ? series.mix.map(run => ({ ...run,
      total: runAt(totals, run.firstHour)?.total ?? null,
      startsSegment: series.segments.some(segment => segment.firstHour === run.firstHour),
    })) : [];
    return [series.tag, { ...series, runs: totals, parts, unknown: reply.types.map(() => null), starts: stretchStarts(totals.map((run, index) => ({
      firstHour: run.firstHour, lastHour: run.lastHour,
      startsSegment: run.startsSegment || run.total === null || totals[index - 1]?.total === null,
    }))) }] as const;
  })), tags: reply.series.map(series => series.tag) };
}

export function countryValuePartsReply(
  reply: CountryValuesReply & { branchId: string; warnings: string[] },
  unresolved: (type: CountryValueType) => boolean = type => type.name === null,
): CountryPartsReply {
  const model = countryValuesModel(reply);
  return { branchId: reply.branchId, warnings: reply.warnings,
    parts: reply.types.map(type => ({ ...type, unresolved: unresolved(type) })),
    series: [...model.series].map(([tag, series]) => {
      const runs: CountryPartsReply['series'][number]['runs'] = [];
      let partIndex = 0;
      for (const total of series.runs) {
        let firstHour = total.firstHour;
        while (firstHour <= total.lastHour) {
          while (partIndex < series.parts.length && series.parts[partIndex].lastHour < firstHour) partIndex++;
          const part = series.parts[partIndex];
          const covered = part && part.firstHour <= firstHour;
          const lastHour = Math.min(total.lastHour,
            covered ? part.lastHour : part ? part.firstHour - 1 : total.lastHour);
          runs.push({ firstHour, lastHour, total: total.total, rawTotal: total.rawTotal,
            rawValues: covered ? part.rawValues : undefined,
            startsSegment: (firstHour === total.firstHour && total.startsSegment)
              || (covered && firstHour === part.firstHour && part.startsSegment)
              || runs.at(-1)?.lastHour !== firstHour - 1,
            values: [...(covered && part.values.length === reply.types.length ? part.values : series.unknown)] });
          firstHour = lastHour + 1;
        }
      }
      return { tag, runs, segments: series.segments, membership: series.membership };
    }) };
}

export function countryValueSegmentAt(model: CountryValuesModel | null, tag: string, hour: number): number | null {
  return runAt(model?.series.get(tag)?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
}

export interface CountryValueAt {
  total: number | null; rawTotal?: string; rawValues?: readonly (string | null)[]; values: readonly (number | null)[]; stretch: number; readHour: number;
}

export function countryValueAt(model: CountryValuesModel, tag: string, hour: number): CountryValueAt | null {
  const series = model.series.get(tag);
  if (!series) return null;
  const position = runIndexAt(series.runs, Math.floor(hour));
  if (position < 0) return null;
  const run = series.runs[position];
  const parts = runAt(series.parts, Math.floor(hour));
  return { total: run.total, rawTotal: run.rawTotal, rawValues: parts?.rawValues,
    values: parts?.values.length === model.reply.types.length ? parts.values : series.unknown,
    stretch: series.starts[position], readHour: Math.floor(hour) };
}

export function rankCountryValues(model: CountryValuesModel, candidates: readonly string[], hour: number, count: number): { tag: string; at: CountryValueAt & { total: number } }[] {
  return candidates.flatMap(tag => {
    const at = countryValueAt(model, tag, hour);
    return at && at.total !== null ? [{ tag, at: { ...at, total: at.total } }] : [];
  }).sort((a, b) => {
    const difference = a.at.rawTotal !== undefined && b.at.rawTotal !== undefined
      ? (BigInt(a.at.rawTotal) < BigInt(b.at.rawTotal) ? 1 : BigInt(a.at.rawTotal) > BigInt(b.at.rawTotal) ? -1 : 0)
      : b.at.total - a.at.total;
    return difference || compareTags(a.tag, b.tag);
  }).slice(0, Math.max(0, count));
}

export function countryValueRuns(model: CountryValuesModel, tag: string, part?: number): Run[] {
  const runs: Run[] = [];
  let broken = true;
  const series = model.series.get(tag);
  for (const run of (part === undefined ? series?.runs : series?.parts) ?? []) {
    const value = part === undefined ? run.total : run.values[part];
    if (value == null) { broken = true; continue; }
    const previous = runs.at(-1);
    const breakBefore = broken || run.startsSegment || (previous !== undefined && previous.lastHour + 1 !== run.firstHour);
    if (previous && !breakBefore && previous.lastHour + 1 === run.firstHour && previous.value === value) previous.lastHour = run.lastHour;
    else runs.push({ firstHour: run.firstHour, lastHour: run.lastHour, value, ...(breakBefore ? { breakBefore: true } : {}) });
    broken = false;
  }
  return runs;
}

export function countryValueMix(model: CountryValuesModel, tag: string, membership: readonly Span[] | null, range: Span): Run[][] {
  return model.reply.types.map((_, index) => shownRuns(countryValueRuns(model, tag, index), membership, range));
}
