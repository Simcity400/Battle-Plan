
import { keyStops } from '../frame.ts';
import type { CountryFlagLook } from '../../components/CountryFlag';
import type { ChartTokens } from '../tokens.ts';

export interface TimelineSpan {
  id: string;
  firstHour: number;
  lastHour: number;
  label: string;
  icon?: string;
  icons?: readonly (string | undefined)[];
  countryFlag?: CountryFlagLook;
  shade?: number;
  emphasis?: boolean;
  marker?: boolean;
  value?: number;
  positiveCount?: number;
  negativeCount?: number;
  dot?: boolean;
  stack?: number;
  positive?: boolean;
  endLabel?: string;
  color?: string;
  glyph?: string;
  glyphHour?: number;
  glyphHours?: readonly number[];
  glyphShape?: 'cross';
  openEnd?: boolean;
  subrow?: number;
  barHeight?: number;
  band?: readonly BandStep[];
  stops?: readonly number[];
}

export interface BandStep { firstHour: number; lastHour: number; value: number }

export interface TimelineLane {
  id: string;
  label: string;
  spans: readonly TimelineSpan[];
  icons?: boolean;
  names?: boolean;
  identity?: { label: string; icon?: string; countryFlag?: CountryFlagLook; tile?: 'blueprint' };
  muted?: boolean;
  stepMax?: number;
  barMax?: number;
  compact?: boolean;
  depth?: number;
  heading?: boolean;
  expanded?: boolean;
  opens?: boolean;
  singleLine?: boolean;
  rowHeight?: number;
  subrows?: number;
  barHeight?: number;
  labelStyle?: 'side';
  separator?: false;
  separatorAfter?: boolean;
  pill?: { text: string; color: string };
  paint?: (ctx: CanvasRenderingContext2D, lane: LaneGeometry, tokens: ChartTokens) => void;
}

export interface LaneGeometry { top: number; height: number; left: number; right: number; hours: readonly [number, number]; toX: (hour: number) => number; playhead: number | null }

export const AXIS_HEIGHT = 24;
export const ROW_HEIGHT = 40;
export const INDENT = 18;

export interface TimelineRows { offsets: Float64Array; rowHeights: Float64Array; height: number }

export function laneRows(lanes: readonly TimelineLane[], defaultHeight = ROW_HEIGHT): TimelineRows {
  const offsets = new Float64Array(lanes.length + 1);
  const rowHeights = new Float64Array(lanes.length);
  for (let index = 0; index < lanes.length; index++) {
    rowHeights[index] = Math.max(1, lanes[index].rowHeight ?? defaultHeight);
    const height = rowHeights[index] * Math.max(1, Math.floor(lanes[index].subrows ?? 1));
    const stacks = stackCount(lanes[index].spans);
    offsets[index + 1] = offsets[index] + Math.max(height, stacks ? stacks * 16 + 8 : 0);
  }
  return { offsets, rowHeights, height: offsets[lanes.length] };
}

function rowBoundary(offsets: Float64Array, position: number, strict = false): number {
  let low = 0;
  let high = offsets.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (offsets[middle] < position || (strict && offsets[middle] === position)) low = middle + 1;
    else high = middle;
  }
  return low;
}

export function laneTop(index: number, rows?: TimelineRows): number {
  return AXIS_HEIGHT + (rows ? rows.offsets[index] : index * ROW_HEIGHT);
}

export function laneLabelY(index: number, rows: TimelineRows, offset: number, viewportHeight: number): number {
  const top = laneTop(index, rows) - offset;
  const height = rows.offsets[index + 1] - rows.offsets[index];
  const middle = top + height / 2;
  const rowHeight = rows.rowHeights[index];
  if (height <= rowHeight) return middle;
  const from = Math.max(AXIS_HEIGHT, top);
  const to = Math.min(viewportHeight, top + height);
  return to - from < rowHeight ? (from + to) / 2 : Math.max(from + rowHeight / 2, Math.min(middle, to - rowHeight / 2));
}

export function laneRowAt(y: number, index: number, rows: TimelineRows): number | null {
  const relative = y - laneTop(index, rows);
  return relative >= 0 && relative < rows.offsets[index + 1] - rows.offsets[index]
    ? Math.floor(relative / rows.rowHeights[index]) : null;
}

export function stackedSpans(spans: readonly TimelineSpan[]): TimelineSpan[] {
  const ends: number[] = [];
  return [...spans].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour || (a.id < b.id ? -1 : 1)).map(span => {
    let stack = ends.findIndex(end => end < span.firstHour);
    if (stack < 0) stack = ends.length;
    ends[stack] = span.lastHour;
    return { ...span, stack };
  });
}

const stackCounts = new WeakMap<readonly TimelineSpan[], number>();

export function stackCount(spans: readonly TimelineSpan[]): number {
  let count = stackCounts.get(spans);
  if (count == null) {
    count = spans.reduce((largest, span) => Math.max(largest, (span.stack ?? -1) + 1), 0);
    stackCounts.set(spans, count);
  }
  return count;
}

export function stackCentre(count: number, stack: number): number {
  return 16 * (stack - (count - 1) / 2);
}

export function spanY(span: TimelineSpan, index: number, rows: TimelineRows, spans: readonly TimelineSpan[]): number {
  const top = laneTop(index, rows);
  return span.stack == null ? top + ((span.subrow ?? 0) + 0.5) * rows.rowHeights[index]
    : top + (rows.offsets[index + 1] - rows.offsets[index]) / 2 + stackCentre(stackCount(spans), span.stack);
}

export function stackedSpanAt(spans: readonly TimelineSpan[], hour: number, offsetFromCentre: number): TimelineSpan | null {
  const count = stackCount(spans);
  return spans.find(span => span.stack != null && span.firstHour <= hour && hour < span.lastHour + 1
    && Math.abs(offsetFromCentre - stackCentre(count, span.stack)) <= 6) ?? null;
}

const pointerBars = new WeakMap<readonly TimelineSpan[], { bars: readonly TimelineSpan[]; stacked: boolean }>();

export function pointerBarSpans(spans: readonly TimelineSpan[], offset: number): readonly TimelineSpan[] {
  let cached = pointerBars.get(spans);
  if (!cached) {
    cached = { bars: spans.some(span => span.marker) ? spans.filter(span => !span.marker) : spans,
      stacked: stackCount(spans) > 0 };
    pointerBars.set(spans, cached);
  }
  if (!cached.stacked) return cached.bars;
  const count = stackCount(spans);
  return cached.bars.filter(span => span.stack == null || Math.abs(offset - stackCentre(count, span.stack)) <= 6);
}

export function markerAt(spans: readonly TimelineSpan[], x: number, toX: (hour: number) => number, from: number, to: number, row?: number): TimelineSpan | null {
  let best: TimelineSpan | null = null, distance = Infinity;
  for (const span of spans) {
    if (!span.marker || span.firstHour < from || span.firstHour >= to || (row != null && (span.subrow ?? 0) !== row)) continue;
    const delta = Math.abs(toX(span.firstHour) - x);
    if (delta <= Math.max(12, (span.icons?.length ?? 1) * 11) && delta < distance) { best = span; distance = delta; }
  }
  return best;
}

export function joinedStep(previous: TimelineSpan | undefined, current: TimelineSpan): boolean {
  return previous?.value != null && current.value != null && previous.lastHour + 1 === current.firstHour;
}

export function periodBarHeight(value: number | null, maximum: number, height = 24): number | null {
  if (value == null || !Number.isFinite(value) || value < 0 || !Number.isFinite(maximum) || maximum <= 0) return null;
  return height * Math.min(value / maximum, 1);
}

export function escapeDismissesTooltip(keyboard: boolean, dismissPointer = false): boolean {
  return keyboard || dismissPointer;
}

export function lanesInView(offset: number, height: number, count: number, rowHeight: number | TimelineRows = ROW_HEIGHT): [number, number] {
  const top = Math.max(0, offset);
  const bottom = offset + Math.max(0, height - AXIS_HEIGHT);
  if (typeof rowHeight !== 'number') return [Math.max(0, rowBoundary(rowHeight.offsets, top, true) - 1), Math.min(count - 1, rowBoundary(rowHeight.offsets, bottom) - 1)];
  return [Math.max(0, Math.floor(top / rowHeight)), Math.min(count - 1, Math.ceil(bottom / rowHeight) - 1)];
}

export function laneOffset(viewTop: number, contentTop: number, height: number, full: number): number {
  return Math.max(0, Math.min(Math.max(0, full - height), viewTop - contentTop));
}

export function laneAt(y: number, count: number, rowHeight: number | TimelineRows = ROW_HEIGHT): number | null {
  if (y < AXIS_HEIGHT) return null;
  const index = typeof rowHeight === 'number' ? Math.floor((y - AXIS_HEIGHT) / rowHeight) : rowBoundary(rowHeight.offsets, y - AXIS_HEIGHT, true) - 1;
  return index >= 0 && index < count ? index : null;
}

export function spanAt(spans: readonly TimelineSpan[], hour: number): TimelineSpan | null {
  return spans.find(span => !span.marker && span.firstHour <= hour && hour < span.lastHour + 1) ?? null;
}

export function spanAtRow(spans: readonly TimelineSpan[], hour: number, row: number): TimelineSpan | null {
  return spans.find(span => !span.marker && span.stack == null && (span.subrow ?? 0) === row && span.firstHour <= hour && hour < span.lastHour + 1) ?? null;
}

export function barHours(span: TimelineSpan, playhead: number | null): [number, number] | null {
  const end = Math.min(span.lastHour + 1, playhead ?? Infinity);
  return span.firstHour < end ? [span.firstHour, end] : null;
}

export function barOpenEnd(span: TimelineSpan, playhead: number | null): boolean {
  return !!span.openEnd && playhead != null && span.firstHour < playhead && playhead < span.lastHour + 1;
}

export function barGlyphHours(span: TimelineSpan, playhead: number | null): readonly number[] {
  return span.glyph || span.glyphShape ? (span.glyphHours ?? [span.glyphHour ?? span.firstHour])
    .filter(hour => hour >= span.firstHour && hour <= span.lastHour && hour <= (playhead ?? Infinity)) : [];
}

export function activateLane(lane: TimelineLane | undefined, key: string, activate?: (lane: TimelineLane) => void): boolean {
  if (!lane || (lane.expanded == null && !lane.opens) || !activate || !['click', 'Enter', ' '].includes(key)) return false;
  activate(lane);
  return true;
}

export function inSpans(spans: readonly { firstHour: number; lastHour: number }[] | undefined, hour: number): boolean {
  return !!spans?.some(span => span.firstHour <= hour && hour <= span.lastHour);
}

export function leftColumn(fitted: number, width: number): number {
  return Math.min(fitted, Math.max(96, Math.floor(width * 0.4)));
}

export interface BandEdge { x0: number; x1: number; half: number }

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

export interface CachedBands { lane: TimelineLane; view: string; bands: LaneBands }

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

export interface BandHit { spanId: string; firstHour: number; lastHour: number }

export interface LaneBands { outlines: Map<string, BandEdge[][]>; columns: Map<number, BandHit> }

export function bandHitAt(bands: LaneBands, x: number, hour: number): { spanId: string; hour: number } | null {
  const hit = bands.columns.get(Math.floor(x));
  return hit ? { spanId: hit.spanId, hour: Math.min(hit.lastHour, Math.max(hit.firstHour, Math.floor(hour))) } : null;
}

interface Column { column: number; half: number; key?: string; firstHour: number; lastHour: number }

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

function bandColumns(steps: readonly (BandStep & { key?: string })[], from: number, to: number, toX: (hour: number) => number, half: (value: number) => number,
  left: number, right: number): Column[][] {
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
  interface Drawn { segment: Segment; half: number; firstHour: number; lastHour: number }
  const drawnAt: (Drawn | undefined)[] = new Array(Math.max(0, lastColumn - firstColumn + 1));
  const get = (column: number) => column >= firstColumn && column <= lastColumn ? drawnAt[column - firstColumn] : undefined;
  const set = (column: number, value: Drawn | undefined) => { drawnAt[column - firstColumn] = value; };
  const at = (segment: Segment, column: number): Drawn => {
    const index = cell(segment, column);
    return { segment, half: thick[index], firstHour: hoursFirst[index], lastHour: hoursLast[index] };
  };
  const coveredLow = runs.map(run => run.x0 <= left ? firstColumn : Math.ceil(run.x0));
  const coveredHigh = runs.map(run => run.x1 >= right ? lastColumn : Math.floor(run.x1) - 1);
  for (const segment of segments) {
    for (let column = Math.max(coveredLow[segment.run], segment.first); column <= Math.min(coveredHigh[segment.run], lastOf(segment)); column++) {
      const current = get(column);
      if (!current || thick[cell(segment, column)] > current.half) set(column, at(segment, column));
    }
  }
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
  const placed = new Map<Segment, { segment: Segment; column: number }>();
  const queue = marks;
  for (let head = 0; head < queue.length; head++) {
    const { segment, column: middle } = queue[head];
    let best = segment.offset;
    for (let index = segment.offset + 1; index < segment.offset + segment.size; index++) if (thick[index] > thick[best]) best = index;
    const thickness = thick[best];
    let chosen: number | null = null;
    let fallback: number | null = null;
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

export function bandHalf(value: number, max: number, least: number, most: number): number {
  if (!(max > 0) || !(value > 0)) return least;
  return least + (most - least) * Math.min(1, value / max);
}

export function barExtent(x0: number, x1: number, left: number, right: number): [number, number] | null {
  if (x1 <= left || x0 >= right) return null;
  const from = Math.max(left, x0 + 1);
  const to = Math.min(right, x1 - 1);
  if (to - from >= 1.5) return [from, to];
  const middle = Math.floor((Math.max(left, x0) + Math.min(right, x1)) / 2);
  const column = Math.min(Math.max(middle, Math.ceil(left)), Math.floor(right) - 1);
  return column < left ? null : [column, column + 1];
}

export function barHitAt(spans: readonly TimelineSpan[], px: number, playhead: number | null, toX: (hour: number) => number,
  hourAt: (px: number) => number, left: number, right: number, row?: number): { span: TimelineSpan; hour: number } | null {
  for (let index = spans.length - 1; index >= 0; index--) {
    const span = spans[index];
    if (span.band || span.marker || (row != null && span.stack == null && (span.subrow ?? 0) !== row)) continue;
    const hours = barHours(span, playhead);
    const extent = hours && barExtent(toX(hours[0]), toX(hours[1]), left, right);
    if (!hours || !extent || px < extent[0] || px >= extent[1]) continue;
    const last = Math.min(span.lastHour, Math.ceil(hours[1]) - 1);
    return { span, hour: Math.min(last, Math.max(span.firstHour, Math.floor(hourAt(px)))) };
  }
  return null;
}

export interface LaneStop {
  id: string;
  hour: number;
  span: TimelineSpan;
}

export function laneStops(spans: readonly TimelineSpan[], from: number, to: number, playhead: number | null): LaneStop[] {
  const end = Math.min(playhead ?? to, to);
  const stops: LaneStop[] = [];
  let order = false;
  for (const span of spans) {
    if (span.marker && (span.firstHour < from || span.firstHour >= end)) continue;
    if (span.firstHour > end || Math.min(span.lastHour + 1, playhead ?? Infinity) <= from) continue;
    const last = Math.max(Math.min(end, span.lastHour), Math.floor(from) <= span.lastHour ? Math.min(from, end) : -Infinity);
    if (span.subrow != null || span.stack != null) order = true;
    const events = span.glyphHour == null && !span.glyphHours ? [] : barGlyphHours(span, playhead);
    for (const event of events) if (event >= from && event <= end && event !== span.firstHour) {
      stops.push({ id: `${span.id}@glyph:${event}`, hour: event, span });
      order = true;
    }
    const band = span.band?.flatMap(step => {
      const firstHour = Math.max(step.firstHour, span.firstHour);
      const lastHour = Math.min(step.lastHour, span.lastHour);
      return firstHour <= lastHour ? [{ ...step, firstHour, lastHour }] : [];
    });
    const starts = band ? band.map(step => step.firstHour) : [span.firstHour];
    const own = span.stops?.filter(hour => hour > span.firstHour && hour <= span.lastHour) ?? [];
    const all = own.length ? mergeHours(starts, own) : starts;
    let index = 0;
    let source = 0;
    for (const hour of keyStops(all, from, last)) {
      const whole = Math.floor(hour);
      if (band) {
        while (index < band.length && band[index].lastHour < whole) index++;
        const step = band[index];
        if (!step || step.firstHour > whole) continue;
      }
      while (source + 1 < all.length && all[source + 1] <= hour) source++;
      stops.push({ id: `${span.id}@${all[source]}`, hour, span });
    }
  }
  if (order) stops.sort((a, b) => a.hour - b.hour || (a.span.subrow ?? a.span.stack ?? 0) - (b.span.subrow ?? b.span.stack ?? 0) || a.id.localeCompare(b.id));
  return stops;
}

function mergeHours(a: readonly number[], b: readonly number[]): number[] {
  const merged: number[] = [];
  let [i, j] = [0, 0];
  while (i < a.length || j < b.length) {
    const next = j >= b.length || (i < a.length && a[i] <= b[j]) ? a[i++] : b[j++];
    if (merged.at(-1) !== next) merged.push(next);
  }
  return merged;
}

export interface HolderState { key: string | null; previous: TimelineSpan | null; current: TimelineSpan | null; t: number }

const holderKey = (span: TimelineSpan | null) => span ? `${span.id}\u0000${span.label}\u0000${span.icon ?? span.countryFlag?.flag ?? ''}` : null;

export function nextHolder(state: HolderState | undefined, current: TimelineSpan | null, fadePresentation = true): HolderState {
  const key = holderKey(current);
  if (!state) return { key, previous: null, current, t: 1 };
  if (!fadePresentation && state.current?.id === current?.id) {
    state.key = key;
    state.current = current;
    return state;
  }
  if (state.key === key) { state.current = current; return state; }
  return { key, previous: state.current, current, t: 0 };
}

