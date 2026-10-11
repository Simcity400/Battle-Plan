import { divergingStack, isSpan, runAt } from '../../charts/series.ts';
import type { Band } from '../../charts/series';
import type { HourSpan, MeasureParts, StabilityPart, StabilityPartsReply } from '../../generated/dashboard';
import { orderedRuns } from '../country-measure/measureData.ts';
import { lineName, rawAt, termLabel } from './stabilityData.ts';
import type { Measure } from './stabilityData';

/** The drill-down of stability or war support: the parts of the
 * value at one day as bars, and the same parts over time as a stacked chart, from
 * the `stability-parts` product. A part is known only where it was recorded. */

/** The part no recorded source explains, as the tooltip names it. */
export const REMAINDER_LABEL = 'Other';
/** Parts past the palette are shown together under this name. */
export const SMALLER_LABEL = 'Smaller parts';

/** Checks a `stability-parts` reply's shape: every part's runs, and the known
 * hours, in hour order and never overlapping. */
export function validateStabilityPartsReply(data: unknown): StabilityPartsReply {
  const reply = data as StabilityPartsReply;
  const measure = (value: MeasureParts) => typeof value === 'object' && value != null
    && Array.isArray(value.observed) && value.observed.every(isSpan)
    && value.observed.every((span, index) => index === 0 || value.observed[index - 1].lastHour < span.firstHour)
    && Array.isArray(value.parts) && value.parts.every(part => typeof part.id === 'string' && orderedRuns(part.runs)
      && (part.term === null || typeof part.term === 'string') && (part.line === null || typeof part.line === 'object'));
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && measure(reply.stability) && measure(reply.warSupport);
  if (!ok) throw new Error('The stability parts reply is malformed.');
  return reply;
}

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
export function partLayers(parts: MeasureParts, measure: Measure, palette: readonly string[], grey: string): PartLayer[] {
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
    sum + Math.abs(Number(run.valueRaw)) * (shownThrough(run.lastHour) - shownThrough(run.firstHour - 1)), 0);
  const weights = new Map(parts.parts.map(part => [part.id, weight(part)]));
  const ranked = parts.parts.filter(part => weights.get(part.id)! > 0)
    .sort((a, b) => weights.get(b.id)! - weights.get(a.id)! || a.id.localeCompare(b.id));
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
