/** Pure layout for lane timelines: which lane and span a point is over, which
 * span holds a lane at an hour, and where a span's bar runs. */

import { keyStops } from '../frame.ts';

export interface TimelineSpan {
  id: string;
  /** First and last recorded hour the holder was observed. */
  firstHour: number;
  lastHour: number;
  label: string;
  /** A PNG data URL of the holder's icon or portrait. */
  icon?: string;
  /** 0..1 along a sequential ramp (a law's escalation); none draws the neutral fill. */
  shade?: number;
  /** Drawn in the accent colour, such as a spirit still active at the playhead. */
  emphasis?: boolean;
  /** A band instead of a bar: its thickness follows a value over the span's hours
   * (such as a production line's factories each day), on the timeline's `bandMax`.
   * Hours no step covers are not drawn. */
  band?: readonly BandStep[];
}

/** A value held over hours, both included: one step of a band. */
export interface BandStep { firstHour: number; lastHour: number; value: number }

export interface TimelineLane {
  id: string;
  /** The slot's name, under the holder in the left column. */
  label: string;
  spans: readonly TimelineSpan[];
  /** Icons and names inside this lane's bars; the chart's settings otherwise. */
  icons?: boolean;
  names?: boolean;
  /** A lane about one thing (a spirit) names it in the left column whatever the
   * playhead holds, with `label` beneath. */
  identity?: { label: string; icon?: string };
  /** Faded in the left column, such as a line that has ended. */
  muted?: boolean;
}

export const AXIS_HEIGHT = 24;
export const ROW_HEIGHT = 40;

/** The lane under a y position, or null above the lanes or below the last. */
export function laneAt(y: number, count: number): number | null {
  if (y < AXIS_HEIGHT) return null;
  const index = Math.floor((y - AXIS_HEIGHT) / ROW_HEIGHT);
  return index >= 0 && index < count ? index : null;
}

/** The span holding a lane at an hour. A span covers its last observed hour up to
 * the next one, so a daily-sampled holder has no gaps between its samples. */
export function spanAt(spans: readonly TimelineSpan[], hour: number): TimelineSpan | null {
  return spans.find(span => span.firstHour <= hour && hour < span.lastHour + 1) ?? null;
}

/** The hours a span's bar covers: never past the playhead, none when it starts after it. */
export function barHours(span: TimelineSpan, playhead: number | null): [number, number] | null {
  const end = Math.min(span.lastHour + 1, playhead ?? Infinity);
  return span.firstHour < end ? [span.firstHour, end] : null;
}

/** Whether an hour falls in one of the given spans (such as the unobserved hours). */
export function inSpans(spans: readonly { firstHour: number; lastHour: number }[] | undefined, hour: number): boolean {
  return !!spans?.some(span => span.firstHour <= hour && hour <= span.lastHour);
}

/** The left column's width: what its longest text needs, but never more than
 * 40% of the chart, so the bars always keep room. */
export function leftColumn(fitted: number, width: number): number {
  return Math.min(fitted, Math.max(96, Math.floor(width * 0.4)));
}

/** One flat stretch of a band's upper edge: from `x0` to `x1` pixels, `half` pixels
 * above the lane's centre (the lower edge mirrors it). */
export interface BandEdge { x0: number; x1: number; half: number }

/** A band as drawn in whole pixel columns between the plot's `left` and `right`
 * pixels. The hours in [`from`, `to`) that a step covers are recorded; every other
 * hour (a missing hour, or before or after the band) is a gap. A run is an unbroken
 * stretch of recorded hours; a segment is the part of a run of one key (a span, in
 * `laneBands`). One rule decides what is drawn:
 *
 * 1. A column that recorded hours cover fully (within the plot) is drawn at the
 *    thickest step touching it, by that step's segment.
 * 2. A column a gap also touches is left empty, so rounding never fills over hours the
 *    recording lacks; the thickest step of each segment in it passes to the nearest
 *    column that same segment draws, never across to another segment, so no peak is
 *    lost or painted in another span's colour.
 * 3. A segment drawing no column is drawn in one, at its thickest, so no band vanishes:
 *    it tries every column its data touches, its middle first, then the others by
 *    distance, and takes the first free one, else one held by a thinner mark (which
 *    then tries its own columns again). It never takes a column another segment draws
 *    by rule 1, and gives up only when each is held by a thicker mark.
 *    A mark beside another run's column with more than a pixel of gap between them
 *    moves one column away from that gap when that column is free and brings it next
 *    to no other; marks are resolved forward, then backward, so at either plot edge
 *    whichever neighbour can move does. Separate runs so never touch across a gap
 *    wider than a pixel, unless neither can move.
 * 4. Where the plot cuts a run is not a gap, and a step with no pixels in view draws
 *    nothing.
 *
 * Adjacent drawn columns of equal thickness join into one stretch, one list per run.
 * Linear in the steps and the plot's columns (the marks placed are sorted once). A lane with
 * several band spans places them together (`laneBands`), so the rule holds between
 * spans too. */
export function bandOutlines(steps: readonly BandStep[], from: number, to: number, toX: (hour: number) => number, half: (value: number) => number,
  left: number, right: number): BandEdge[][] {
  return bandColumns(steps, from, to, toX, half, left, right).map(run => stretches(run, left, right, false)).map(groups => groups.flat());
}

/** A lane's bands, placed together: every band span's steps (each cut at its bar's end,
 * `barHours`) go through `bandOutlines`' rule in one pass, so separate spans are runs
 * separated by gaps and never touch across a gap wider than a pixel, however they are
 * split. Each drawn column belongs to the span of the step whose value it draws; the
 * outlines come back by span id, for each span to be filled in its own colour, and each
 * drawn column with the span and hours it draws, so hover and Jump to describe what is
 * drawn under the pointer (`bandHitAt`), even a short band moved off its own hours. */
export function laneBands(spans: readonly TimelineSpan[], playhead: number | null, toX: (hour: number) => number, half: (value: number) => number,
  left: number, right: number): LaneBands {
  const steps: (BandStep & { key: string })[] = [];
  for (const span of [...spans].sort((a, b) => a.firstHour - b.firstHour)) {
    const hours = span.band && barHours(span, playhead);
    if (!span.band || !hours) continue;
    for (const step of span.band) {
      const first = Math.max(step.firstHour, hours[0]);
      const last = Math.min(step.lastHour + 1, hours[1]) - 1;
      if (last + 1 > first) steps.push({ firstHour: first, lastHour: last, value: step.value, key: span.id });
    }
  }
  const outlines = new Map<string, BandEdge[][]>();
  const columns = new Map<number, BandHit>();
  for (const run of bandColumns(steps, -Infinity, Infinity, toX, half, left, right)) {
    for (const column of run) columns.set(column.column, { spanId: column.key!, firstHour: column.firstHour, lastHour: column.lastHour });
    for (const group of stretches(run, left, right, true)) {
      const key = group.key!;
      const list = outlines.get(key) ?? [];
      list.push([...group]);
      outlines.set(key, list);
    }
  }
  return { outlines, columns };
}

/** A lane's band layout as last computed, and the lane and view it was computed for. */
export interface CachedBands { lane: TimelineLane; view: string; bands: LaneBands }

/** The band layouts of the lanes shown, by lane id: only lanes with bands, so a lane
 * that is gone is dropped. A lane whose object and view (`view`: whatever the layout
 * depends on besides the lane, such as the hours in view, the plot's pixels, the
 * playhead and the band scale) are unchanged reuses its previous layout, so moving the
 * pointer lays nothing out again. */
export function laneBandLayouts(lanes: readonly TimelineLane[], view: string, previous: ReadonlyMap<string, CachedBands>,
  layout: (lane: TimelineLane) => LaneBands): Map<string, CachedBands> {
  const next = new Map<string, CachedBands>();
  for (const lane of lanes) {
    if (!lane.spans.some(span => span.band)) continue;
    const cached = previous.get(lane.id);
    next.set(lane.id, cached && cached.lane === lane && cached.view === view ? cached : { lane, view, bands: layout(lane) });
  }
  return next;
}

/** What a drawn band column shows: the span, and the hours of the step it draws. */
export interface BandHit { spanId: string; firstHour: number; lastHour: number }

/** A lane's band outlines by span id, and what each drawn pixel column shows. */
export interface LaneBands { outlines: Map<string, BandEdge[][]>; columns: Map<number, BandHit> }

/** What the band under the pointer at `x` pixels (hour `hour` there) describes, as the
 * strip resolves its bars by pixel column (`stripColumns`): the span its column draws, at the pointer's
 * hour when that step covers it, else at the nearest hour of the step, so a short band
 * drawn off its own hours is described and jumped to at them. Null over no drawn column. */
export function bandHitAt(bands: LaneBands, x: number, hour: number): { spanId: string; hour: number } | null {
  const hit = bands.columns.get(Math.floor(x));
  return hit ? { spanId: hit.spanId, hour: Math.min(hit.lastHour, Math.max(hit.firstHour, Math.floor(hour))) } : null;
}

/** A drawn column: its thickness, and the key (span) and hours of the step it draws. */
interface Column { column: number; half: number; key?: string; firstHour: number; lastHour: number }

/** Adjacent columns joined into stretches of equal thickness, and when `byKey`, split
 * where the key changes; kept to the plot and at least a pixel wide at either end. */
function stretches(columns: readonly Column[], left: number, right: number, byKey: boolean): (BandEdge[] & { key?: string })[] {
  const groups: (BandEdge[] & { key?: string })[] = [];
  for (const { column, half, key } of columns) {
    let group = groups.at(-1);
    if (!group || (byKey && group.key !== key) || group[group.length - 1].x1 !== column) {
      group = Object.assign([] as BandEdge[], { key });
      groups.push(group);
    }
    const previous = group.at(-1);
    if (previous && previous.x1 === column && previous.half === half) previous.x1 = column + 1;
    else group.push({ x0: column, x1: column + 1, half });
  }
  const first = groups[0];
  const last = groups.at(-1);
  if (first) first[0].x0 = Math.max(left, first[0].x0);
  if (last) {
    const tail = last[last.length - 1];
    tail.x1 = Math.max(tail.x0 + 1, Math.min(right, tail.x1));
  }
  return groups;
}

/** `bandOutlines`' rule: the drawn columns of each run that draws any, in order. Linear
 * in the steps and the plot's columns (plus one sort of the marks placed): pieces and
 * per-column values live in flat arrays, and the output is one sweep of the columns. */
function bandColumns(steps: readonly (BandStep & { key?: string })[], from: number, to: number, toX: (hour: number) => number, half: (value: number) => number,
  left: number, right: number): Column[][] {
  // A stretch of one key (span) within a run: its pieces [start, end), and its columns
  // from `first`, `size` of them, at `offset` in the per-column arrays.
  interface Segment { x0: number; x1: number; key?: string; run: number; start: number; end: number; first: number; size: number; offset: number }
  const count = steps.length;
  const pieceX0 = new Float64Array(count);
  const pieceX1 = new Float64Array(count);
  const pieceHalf = new Float64Array(count);
  const pieceFirst = new Float64Array(count);
  const pieceLast = new Float64Array(count);
  let pieces = 0;
  const runs: { x0: number; x1: number }[] = [];
  const segments: Segment[] = [];
  let end: number | null = null;
  for (const step of steps) {
    const a = Math.max(step.firstHour, from);
    const b = Math.min(step.lastHour + 1, to);
    if (b <= a) continue;
    const joins = end === a;
    end = b;
    const x0 = Math.max(left, toX(a));
    const x1 = Math.min(right, toX(b));
    if (x1 <= x0) continue;
    const previous = segments.length ? segments[segments.length - 1] : null;
    const joined = joins && previous != null && previous.x1 === x0;
    if (joined && previous.key === step.key) {
      previous.x1 = x1;
      previous.end = pieces + 1;
    } else {
      const run = joined ? previous.run : runs.push({ x0, x1 }) - 1;
      segments.push({ x0, x1, key: step.key, run, start: pieces, end: pieces + 1, first: 0, size: 0, offset: 0 });
    }
    runs[segments[segments.length - 1].run].x1 = x1;
    pieceX0[pieces] = x0;
    pieceX1[pieces] = x1;
    pieceHalf[pieces] = half(step.value);
    pieceFirst[pieces] = Math.ceil(a);
    pieceLast[pieces] = Math.ceil(b) - 1;
    pieces++;
  }
  const firstColumn = Math.floor(left);
  const lastColumn = Math.ceil(right) - 1;
  // Each segment's thickest step in each column it touches, and that step's hours.
  let total = 0;
  for (const segment of segments) {
    segment.first = Math.floor(segment.x0);
    segment.size = Math.max(1, Math.ceil(segment.x1) - segment.first);
    segment.offset = total;
    total += segment.size;
  }
  const thick = new Float64Array(total).fill(-Infinity);
  const hoursFirst = new Float64Array(total);
  const hoursLast = new Float64Array(total);
  for (const segment of segments) {
    for (let piece = segment.start; piece < segment.end; piece++) {
      const first = Math.floor(pieceX0[piece]);
      const last = Math.max(first, Math.ceil(pieceX1[piece]) - 1);
      for (let column = first; column <= last; column++) {
        const index = segment.offset + column - segment.first;
        if (pieceHalf[piece] > thick[index]) { thick[index] = pieceHalf[piece]; hoursFirst[index] = pieceFirst[piece]; hoursLast[index] = pieceLast[piece]; }
      }
    }
  }
  const lastOf = (segment: Segment) => segment.first + segment.size - 1;
  const cell = (segment: Segment, column: number) => segment.offset + column - segment.first;
  // What each drawn column shows: the segment, the thickness, and the hours of the step it is from.
  interface Drawn { segment: Segment; half: number; firstHour: number; lastHour: number }
  const drawnAt: (Drawn | undefined)[] = new Array(Math.max(0, lastColumn - firstColumn + 1));
  const get = (column: number) => column >= firstColumn && column <= lastColumn ? drawnAt[column - firstColumn] : undefined;
  const set = (column: number, value: Drawn | undefined) => { drawnAt[column - firstColumn] = value; };
  const at = (segment: Segment, column: number): Drawn => {
    const index = cell(segment, column);
    return { segment, half: thick[index], firstHour: hoursFirst[index], lastHour: hoursLast[index] };
  };
  // Fully recorded columns of each run: inside it, or cut only by the plot (rule 4).
  const coveredLow = runs.map(run => run.x0 <= left ? firstColumn : Math.ceil(run.x0));
  const coveredHigh = runs.map(run => run.x1 >= right ? lastColumn : Math.floor(run.x1) - 1);
  // Rule 1: each fully recorded column, drawn by the segment with the thickest step there.
  for (const segment of segments) {
    for (let column = Math.max(coveredLow[segment.run], segment.first); column <= Math.min(coveredHigh[segment.run], lastOf(segment)); column++) {
      const current = get(column);
      if (!current || thick[cell(segment, column)] > current.half) set(column, at(segment, column));
    }
  }
  // Rule 2: a column a gap touches stays empty; its thickest passes to the nearest column
  // its own segment draws, never across to another span. Rule 3: a segment drawing no
  // column gets a mark, placed below.
  const marks: { segment: Segment; column: number }[] = [];
  for (const segment of segments) {
    const low = coveredLow[segment.run];
    const high = coveredHigh[segment.run];
    let firstOwned = -1;
    let lastOwned = -1;
    for (let column = segment.first; column <= lastOf(segment); column++) {
      if (get(column)?.segment !== segment) continue;
      if (firstOwned < 0) firstOwned = column;
      lastOwned = column;
    }
    if (firstOwned < 0) {
      marks.push({ segment, column: Math.min(lastOf(segment), Math.max(segment.first, Math.floor((segment.x0 + segment.x1) / 2))) });
      continue;
    }
    for (let column = segment.first; column <= lastOf(segment); column++) {
      if (column >= low && column <= high) continue;
      const target = get(column < firstOwned ? firstOwned : lastOwned)!;
      if (thick[cell(segment, column)] > target.half) Object.assign(target, at(segment, column));
    }
  }
  // A mark tries every column its data touches, its middle first and then the others by
  // distance from it, and takes the first free one, else the first held by a thinner
  // mark (which then tries its own columns again); it never takes a column another
  // segment draws by rule 1, and gives up only when every one is held by a thicker mark.
  // The queue is read by index (a displaced mark joins its end), so each entry is O(1).
  const placed = new Map<Segment, { segment: Segment; column: number }>();
  const queue = marks;
  for (let head = 0; head < queue.length; head++) {
    const { segment, column: middle } = queue[head];
    let best = segment.offset;
    for (let index = segment.offset + 1; index < segment.offset + segment.size; index++) if (thick[index] > thick[best]) best = index;
    const thickness = thick[best];
    let chosen: number | null = null;
    let fallback: number | null = null;
    // Its middle, then one column left and right, then two, and so on.
    for (let step = 0; step <= 2 * segment.size; step++) {
      const distance = (step + 1) >> 1;
      const column = step === 0 ? middle : step % 2 ? middle - distance : middle + distance;
      if (column < segment.first || column > lastOf(segment)) continue;
      const occupant = get(column);
      if (!occupant) { chosen = column; break; }
      if (fallback == null && placed.has(occupant.segment) && occupant.half < thickness) fallback = column;
    }
    const column = chosen ?? fallback;
    if (column == null) continue;
    const occupant = get(column);
    if (occupant) {
      placed.delete(occupant.segment);
      queue.push({ segment: occupant.segment, column });
    }
    set(column, { segment, half: thickness, firstHour: hoursFirst[best], lastHour: hoursLast[best] });
    placed.set(segment, { segment, column });
  }
  // Separate runs never touch across more than a pixel of gap: a mark next to another
  // run's column beyond such a gap moves one column away from it, if that column is
  // free and brings it next to no other; forward, then backward, so at either plot edge
  // the neighbour that can move does.
  const apart = (segment: Segment, column: number, side: -1 | 1) => {
    const other = get(column)?.segment;
    if (!other || other === segment) return false;
    return side < 0 ? segment.x0 - other.x1 > 1 : other.x0 - segment.x1 > 1;
  };
  const free = (column: number) => column >= firstColumn && column <= lastColumn && !get(column);
  const resolve = (mark: { segment: Segment; column: number }) => {
    const { segment, column } = mark;
    const move = (target: number) => { set(target, get(column)); set(column, undefined); mark.column = target; };
    if (apart(segment, column - 1, -1) && free(column + 1) && !apart(segment, column + 2, 1)) move(column + 1);
    else if (apart(segment, column + 1, 1) && free(column - 1) && !apart(segment, column - 2, -1)) move(column - 1);
  };
  const order = [...placed.values()].sort((a, b) => a.column - b.column);
  for (const mark of order) resolve(mark);
  for (const mark of [...order].reverse()) resolve(mark);
  // One sweep of the plot's columns: each run's drawn columns, in order.
  const output: Column[][] = [];
  let run = -1;
  for (let column = firstColumn; column <= lastColumn; column++) {
    const entry = drawnAt[column - firstColumn];
    if (!entry) continue;
    if (entry.segment.run !== run) { output.push([]); run = entry.segment.run; }
    output[output.length - 1].push({ column, half: entry.half, key: entry.segment.key, firstHour: entry.firstHour, lastHour: entry.lastHour });
  }
  return output;
}

/** A band value's half-thickness in a lane: from `least` (so a running zero still
 * shows) up to `most` at `max`. */
export function bandHalf(value: number, max: number, least: number, most: number): number {
  if (!(max > 0) || !(value > 0)) return least;
  return least + (most - least) * Math.min(1, value / max);
}

/** Where the keyboard can stop in a lane: a bar's first hour shown, or, for a band,
 * each step's first hour shown, so every value a pointer can reach is reachable. */
export interface LaneStop {
  /** The span and the step (its first hour) the stop belongs to, stable through zoom. */
  id: string;
  hour: number;
  span: TimelineSpan;
}

/** A lane's keyboard stops in the view from `from` to `to`, up to the playhead, in
 * hour order: for each span shown, its stops by the kit's `keyStops` (those in view,
 * and the view's start when an earlier stop carries its value into it; a stop is
 * described at its whole hour, as hover floors it). A span covering the view's first,
 * partial hour is reachable there even when that hour is its last. A band's stops are
 * its steps' first hours within the span (steps cut to it, as drawn); an hour no step
 * covers is not a stop. Linear in the steps. */
export function laneStops(spans: readonly TimelineSpan[], from: number, to: number, playhead: number | null): LaneStop[] {
  const end = Math.min(playhead ?? to, to);
  const stops: LaneStop[] = [];
  for (const span of spans) {
    // A bar is drawn from its first hour to its end, cut at the playhead.
    if (span.firstHour > end || Math.min(span.lastHour + 1, playhead ?? Infinity) <= from) continue;
    // A span whose last hour is the view's first, partial hour reaches up to the view's
    // start: hover there shows that hour, so its carried stop is the view's start.
    const last = Math.max(Math.min(end, span.lastHour), Math.floor(from) <= span.lastHour ? Math.min(from, end) : -Infinity);
    if (!span.band) {
      for (const hour of keyStops([span.firstHour], from, last)) stops.push({ id: `${span.id}@${span.firstHour}`, hour, span });
      continue;
    }
    // Steps cut to the span's hours, as its band is drawn (`laneBands`). The stops and the
    // steps are both in hour order: one walk pairs each stop with the step holding its
    // hour (as hover floors it), so this is linear in the steps.
    const band = span.band.flatMap(step => {
      const firstHour = Math.max(step.firstHour, span.firstHour);
      const lastHour = Math.min(step.lastHour, span.lastHour);
      return firstHour <= lastHour ? [{ ...step, firstHour, lastHour }] : [];
    });
    let index = 0;
    for (const hour of keyStops(band.map(step => step.firstHour), from, last)) {
      const whole = Math.floor(hour);
      while (index < band.length && band[index].lastHour < whole) index++;
      const step = band[index];
      if (step && step.firstHour <= whole) stops.push({ id: `${span.id}@${step.firstHour}`, hour, span });
    }
  }
  return stops;
}

/** Every span's hours together, for fitting the time axis. */
export function spansExtent(lanes: readonly TimelineLane[]): [number, number] | null {
  let first = Infinity;
  let last = -Infinity;
  for (const lane of lanes) {
    for (const span of lane.spans) {
      first = Math.min(first, span.firstHour);
      last = Math.max(last, span.lastHour + 1);
    }
  }
  return first <= last ? [first, last] : null;
}
