import { divergingStack, runAt } from '../../charts/series.ts';
import type { Band } from '../../charts/series';
import type { HourSpan } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import type { KeyedState, SeriesPart } from '../../dashboard/keyedSeriesData';
import type { FixedRun, MeasureParts, StabilityPart, StabilityPartName, StabilityPartNamesReply } from './types';
import { lineName, rawAt, termLabel } from './stabilityData.ts';
import type { Measure } from './stabilityData';

/** The drill-down of stability or war support: the parts of the
 * value at one day as bars, and the same parts over time as a stacked chart, from
 * the `stability-parts` product. A part is known only where it was recorded. */

/** The part no recorded source explains, as the tooltip names it. */
export const REMAINDER_LABEL = 'Other';
/** Parts past the palette are shown together under this name. */
export const SMALLER_LABEL = 'Smaller parts';

/** Parts added as the engine adds them: in wrapping signed 64-bit arithmetic,
 * so the total is the value the game shows even at the extremes. */
export function engineSum(raws: readonly string[]): bigint {
  return raws.reduce((total, raw) => BigInt.asIntN(64, total + BigInt(raw)), 0n);
}

/** A part's name: a formula term's, a line's as the tooltip names it, or what
 * the lines do not explain. `recorded` marks the game's own text. */
export function partName(part: StabilityPart, measure: Measure): { name: string; recorded: boolean } {
  if (part.line) return lineName(part.line);
  if (part.term) return { name: termLabel(measure, part.term), recorded: false };
  if (part.id === 'clamp') return { name: 'Limited to the allowed range', recorded: false };
  return { name: REMAINDER_LABEL, recorded: false };
}

/** Whether the parts are known at an hour. */
export function knownAt(parts: MeasureParts, hour: number): boolean {
  return runAt(parts.observed, hour) != null;
}

/** Each part's value at an hour (a part without a run there is zero), or null
 * when the parts are not known then. */
export function partsAt(parts: MeasureParts, hour: number): { part: StabilityPart; raw: string }[] | null {
  if (!knownAt(parts, hour)) return null;
  return parts.parts.flatMap(part => {
    const raw = rawAt(part.runs, hour);
    return raw == null || raw === '0' ? [] : [{ part, raw }];
  });
}

/** The parts known only within `spans` (a group's membership, the widget's
 * range): outside them the country is not shown, so nothing is known there. */
export function partsWithin(parts: MeasureParts, spans: readonly HourSpan[] | null): MeasureParts {
  if (!spans) return parts;
  // Both lists in hour order, swept together once.
  const known = [...parts.observed].sort((a, b) => a.firstHour - b.firstHour);
  const shown = [...spans].sort((a, b) => a.firstHour - b.firstHour);
  const observed: HourSpan[] = [];
  let i = 0;
  let j = 0;
  while (i < known.length && j < shown.length) {
    const firstHour = Math.max(known[i].firstHour, shown[j].firstHour);
    const lastHour = Math.min(known[i].lastHour, shown[j].lastHour);
    if (firstHour <= lastHour) observed.push({ firstHour, lastHour });
    if (known[i].lastHour < shown[j].lastHour) i++; else j++;
  }
  return { ...parts, observed };
}

/** The samples of a segment that draw [from, to]: those inside, and the one
 * either side, so a band crossing the edge is drawn to it. Null when none. */
function visibleRange(hours: readonly number[], from: number, to: number): [number, number] | null {
  // A view lying wholly past the cutoff (from after to) shows nothing.
  if (from > to || !hours.length || hours[hours.length - 1] < from || hours[0] > to) return null;
  let low = 0;
  let high = hours.length - 1;
  while (low < high) { const middle = (low + high + 1) >> 1; if (hours[middle] <= from) low = middle; else high = middle - 1; }
  const first = low;
  low = first;
  high = hours.length - 1;
  while (low < high) { const middle = (low + high) >> 1; if (hours[middle] >= to) high = middle; else low = middle + 1; }
  return [first, low];
}

/** Only the samples that draw [from, to], so a zoomed view never builds paths
 * for the whole campaign. */
export function visibleSegment(segment: PartsSegment, from: number, to: number): PartsSegment | null {
  const range = visibleRange(segment.hours, from, to);
  if (!range) return null;
  const [first, last] = range;
  return { hours: segment.hours.slice(first, last + 1), layers: segment.layers,
    bands: segment.bands.map(band => ({ low: band.low.slice(first, last + 1), high: band.high.slice(first, last + 1) })) };
}

/** The lowest and highest stacked edge over [from, to], counting the samples
 * either side, so a band that crosses the whole view still counts. */
export function stackRange(segments: readonly PartsSegment[], from: number, to: number): [number, number] | null {
  let low = Infinity;
  let high = -Infinity;
  for (const segment of segments) {
    const range = visibleRange(segment.hours, from, to);
    if (!range) continue;
    const [first, last] = range;
    for (let index = first; index <= last; index++) {
      for (const band of segment.bands) { low = Math.min(low, band.low[index]); high = Math.max(high, band.high[index]); }
    }
  }
  return low <= high ? [low, high] : null;
}

/** A layer of the stacked chart: one part, or the smaller ones together. */
export interface PartLayer { id: string; name: string; color: string; members: readonly string[] }

/** The parts ranked by how much they contributed (size times duration), the
 * largest in the palette's colours in its order, the rest as one grey layer, so
 * no colour is made up and none repeats. */
export function partLayers(parts: MeasureParts, measure: Measure, palette: readonly string[], grey: string, exactWeights?: ReadonlyMap<string, bigint>): PartLayer[] {
  // Only the known hours count (the widget's range and membership): hours shown before
  // each span starts, so a run's hours in view are a difference of two lookups.
  const observed = [...parts.observed].sort((a, b) => a.firstHour - b.firstHour);
  const before: number[] = [];
  let total = 0;
  for (const span of observed) { before.push(total); total += span.lastHour - span.firstHour + 1; }
  const shownThrough = (hour: number) => {
    let low = 0;
    let high = observed.length - 1;
    let found = -1;
    while (low <= high) { const middle = (low + high) >> 1; if (observed[middle].firstHour <= hour) { found = middle; low = middle + 1; } else high = middle - 1; }
    return found < 0 ? 0 : before[found] + Math.min(hour, observed[found].lastHour) - observed[found].firstHour + 1;
  };
  const weight = (part: StabilityPart) => part.runs.reduce((sum, run) =>
    sum + (BigInt(run.valueRaw) < 0n ? -BigInt(run.valueRaw) : BigInt(run.valueRaw)) * BigInt(shownThrough(run.lastHour) - shownThrough(run.firstHour - 1)), 0n);
  const weights = exactWeights ?? new Map(parts.parts.map(part => [part.id, weight(part)]));
  const ranked = parts.parts.filter(part => (weights.get(part.id) ?? 0n) > 0n)
    .sort((a, b) => weights.get(a.id)! === weights.get(b.id)! ? a.id.localeCompare(b.id) : weights.get(a.id)! > weights.get(b.id)! ? -1 : 1);
  // Every palette colour is used before parts are folded together into grey.
  const room = Math.min(ranked.length, palette.length);
  const layers: PartLayer[] = ranked.slice(0, room).map((part, index) => ({ id: part.id, name: partName(part, measure).name, color: palette[index], members: [part.id] }));
  const rest = ranked.slice(room);
  if (rest.length) layers.push({ id: '__smaller', name: rest.length === 1 ? partName(rest[0], measure).name : SMALLER_LABEL, color: grey, members: rest.map(part => part.id) });
  return layers;
}

/** One stretch of known hours: the hours sampled and each layer's band. */
/** One stretch of known hours: the hours sampled, each band, and the layer each
 * band belongs to (a layer of several parts has a band above zero and one below). */
export interface PartsSegment { hours: number[]; bands: Band[]; layers: number[] }

/** The stacked parts inside [from, to], per stretch of known hours, sampled at
 * each hour a part changes. Positive parts stack up from zero and negative ones
 * down, in layer order. Values are in percent. */
export function partsStack(parts: MeasureParts, layers: readonly PartLayer[], decimals: number, from: number, to: number): PartsSegment[] {
  const layerOf = new Map(layers.flatMap((layer, index) => layer.members.map(id => [id, index] as const)));
  // Every run of a layered part, by start: swept once across the known stretches,
  // each stretch touching only the runs that overlap it. A layer's positive and
  // negative parts are separate rows, so parts of opposite sign never cancel.
  const all = parts.parts.flatMap(part => {
    const layer = layerOf.get(part.id);
    if (layer == null) return [];
    return part.runs.flatMap(run => {
      const raw = Number(run.valueRaw);
      return raw === 0 ? [] : [{ first: run.firstHour, last: run.lastHour, raw, row: layer * 2 + (raw > 0 ? 0 : 1) }];
    });
  }).sort((a, b) => a.first - b.first);
  let next = 0;
  let active: typeof all = [];
  const segments: PartsSegment[] = [];
  for (const span of parts.observed) {
    const first = Math.max(span.firstHour, Math.floor(from));
    const last = Math.min(span.lastHour, Math.ceil(to));
    if (first > last) continue;
    while (next < all.length && all[next].first <= last) active.push(all[next++]);
    active = active.filter(run => run.last >= first);
    // Each value holds from its run's first hour to its last: both ends are sampled.
    const cuts = new Set([first, last]);
    for (const run of active) {
      if (run.first >= first) { cuts.add(run.first); if (run.first > first) cuts.add(run.first - 1); }
      if (run.last <= last) { cuts.add(run.last); if (run.last < last) cuts.add(run.last + 1); }
    }
    const hours = [...cuts].sort((a, b) => a - b);
    // Each run adds its value where it starts in this stretch and takes it away after it ends.
    const changes = active.flatMap(run => [{ hour: Math.max(first, run.first), row: run.row, raw: run.raw }, { hour: run.last + 1, row: run.row, raw: -run.raw }])
      .sort((a, b) => a.hour - b.hour);
    const totals = new Array<number>(layers.length * 2).fill(0);
    const sums = totals.map(() => hours.map(() => 0));
    let change = 0;
    hours.forEach((hour, index) => {
      while (change < changes.length && changes[change].hour <= hour) { totals[changes[change].row] += changes[change].raw; change++; }
      totals.forEach((total, row) => { sums[row][index] = total / 10 ** decimals * 100; });
    });
    const rows = sums.map((values, row) => [row >> 1, values] as const)
      .filter(([, values]) => values.some(value => value !== 0));
    segments.push({ hours, bands: divergingStack(rows.map(([, values]) => values)), layers: rows.map(([layer]) => layer) });
  }
  return segments;
}

/** A part of the `stability-parts` series names its measure in the bits above its handle. */
export function measureOf(part: string): Measure {
  return BigInt(part) >> 32n === 1n ? 'warSupport' : 'stability';
}

const DECIMALS = 5;
const rawOf = (value: number) => String(Math.round(value * 10 ** DECIMALS));
const nameOf = (part: string, names: ReadonlyMap<string, StabilityPartName>) => names.get(part);
const partOf = (part: string, names: ReadonlyMap<string, StabilityPartName>): StabilityPart => {
  const name = nameOf(part, names);
  return { id: name?.id ?? `part:${part}`, term: name?.term ?? null, line: name?.line ?? null, runs: [] };
};
const isTotal = (part: string, names: ReadonlyMap<string, StabilityPartName>) => nameOf(part, names)?.kind === 'total';

function addRun(runs: FixedRun[], run: FixedRun) {
  const previous = runs.at(-1);
  if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.valueRaw === run.valueRaw) previous.lastHour = run.lastHour;
  else runs.push(run);
}

/** One measure's parts over a chart's view from its parts' lines (`parts` in key order, `lines` by part): a bin is
 * drawn at its first hour's value, so the bin holding the head never shows an hour after it; the parts are known
 * where the displayed value's line holds a bin. With `head`, a tiled line's bins stop at the head's bin, which holds
 * the line's value at the head (`valueAt`: the followed value while following) from its first hour to the head.
 * One source under several handles is one part. */
export function partsFromLines(parts: readonly string[], lines: ReadonlyMap<string, ChartSummary | undefined>,
  names: ReadonlyMap<string, StabilityPartName>, head: number | null = null): MeasureParts {
  const observed: HourSpan[] = [];
  const byId = new Map<string, StabilityPart>();
  // A line's runs: its bins before the head's, then the head's own at the value there.
  const runsOf = (line: ChartSummary): FixedRun[] => {
    const cut = head !== null && line.tiles ? Math.floor(head / line.tiles.binHours) * line.tiles.binHours : Infinity;
    const runs: FixedRun[] = [];
    for (const bucket of line.buckets) {
      if (bucket && bucket.firstHour < cut) runs.push({ firstHour: bucket.firstHour, lastHour: Math.min(bucket.lastHour, cut - 1), valueRaw: rawOf(bucket.firstValue), startsSegment: bucket.firstBreak });
    }
    const value = cut === Infinity || head === null ? null : line.tiles!.valueAt(head);
    if (value !== null && Number.isFinite(value) && cut <= head!) {
      const before = runs.at(-1);
      runs.push({ firstHour: cut, lastHour: Math.floor(head!), valueRaw: rawOf(value), startsSegment: !before || before.lastHour + 1 !== cut });
    }
    return runs;
  };
  for (const part of parts) {
    const line = lines.get(part);
    if (!line) continue;
    if (isTotal(part, names)) {
      for (const run of runsOf(line)) {
        const last = observed.at(-1);
        if (last && !run.startsSegment && last.lastHour + 1 === run.firstHour) last.lastHour = run.lastHour;
        else observed.push({ firstHour: run.firstHour, lastHour: run.lastHour });
      }
      continue;
    }
    const made = partOf(part, names);
    const held = byId.get(made.id) ?? made;
    byId.set(made.id, held);
    for (const run of runsOf(line)) addRun(held.runs, run);
  }
  for (const part of byId.values()) part.runs.sort((a, b) => a.firstHour - b.firstHour);
  return { parts: [...byId.values()], observed: observed.sort((a, b) => a.firstHour - b.firstHour) };
}

/** One measure's parts at `hour` from the states read there: known only where the displayed value was recorded. */
export function partsAtStates(parts: readonly string[], states: readonly KeyedState[], hour: number,
  names: ReadonlyMap<string, StabilityPartName>): MeasureParts {
  const at = new Map(states.filter(state => state.hour === hour).map(state => [state.part, state]));
  const known = parts.some(part => isTotal(part, names) && at.get(part)?.run);
  if (!known) return { parts: [], observed: [] };
  const byId = new Map<string, StabilityPart>();
  for (const part of parts) {
    const run = at.get(part)?.run;
    if (!run || isTotal(part, names)) continue;
    const made = partOf(part, names);
    const held = byId.get(made.id);
    if (held) held.runs[0].valueRaw = String(BigInt.asIntN(64, BigInt(held.runs[0].valueRaw) + BigInt(run.valueRaw)));
    else byId.set(made.id, { ...made, runs: [{ firstHour: hour, lastHour: hour, valueRaw: run.valueRaw, startsSegment: false }] });
  }
  return { parts: [...byId.values()], observed: [{ firstHour: hour, lastHour: hour }] };
}

const text = (value: unknown) => value === null || typeof value === 'string';
export function validateStabilityPartNames(value: unknown): StabilityPartNamesReply {
  const reply = value as StabilityPartNamesReply;
  const ok = typeof reply === 'object' && reply !== null && typeof reply.tag === 'string' && Number.isSafeInteger(reply.hour)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string') && Array.isArray(reply.parts)
    && reply.parts.every(part => typeof part === 'object' && part !== null && typeof part.part === 'string' && /^\d+$/.test(part.part)
      && (part.measure === 'stability' || part.measure === 'warSupport') && typeof part.kind === 'string' && text(part.id) && text(part.term)
      && text(part.missing) && (part.line === null || typeof part.line === 'object'));
  if (!ok) throw new Error('The stability part names reply is malformed.');
  return reply;
}

/** The parts to name at each hour, ascending: each part at the last hour it was held up to `end`, where the parts
 * dictionary still holds it. */
export function namesAt(listed: readonly SeriesPart[], end: number): [number, string[]][] {
  const last = new Map<string, number>();
  for (const part of listed) last.set(part.part, Math.max(last.get(part.part) ?? -Infinity, Math.min(part.lastHour, end)));
  const byHour = new Map<number, string[]>();
  for (const [part, hour] of last) byHour.set(hour, [...(byHour.get(hour) ?? []), part]);
  return [...byHour].sort(([a], [b]) => a - b);
}
