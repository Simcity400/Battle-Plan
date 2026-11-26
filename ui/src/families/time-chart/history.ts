import type { ChartBucket } from '../../charts/chartSummary.ts';
import type { Run } from '../../charts/series.ts';
import type { BoundedSeries } from '../plan.ts';

export interface LineSink {
  move(hour: number, value: number): void;
  line(hour: number, value: number): void;
}

export interface LineWindow {
  readonly from: number;
  readonly to: number;
}

export class LineTrace {
  open = false;
  lastHour = 0;
  lastValue = 0;
  ended = false;
  prevLastHour = -Infinity;

  private step: boolean;
  private scale: number;
  private binHours: number;
  private sink: LineSink;

  constructor(step: boolean, scale: number, binHours: number, sink: LineSink) {
    this.step = step;
    this.scale = scale;
    this.binHours = binHours;
    this.sink = sink;
  }

  configure(step: boolean, scale: number, binHours: number, sink: LineSink): void {
    this.step = step;
    this.scale = scale;
    this.binHours = binHours;
    this.sink = sink;
    this.open = false;
    this.lastHour = 0;
    this.lastValue = 0;
    this.ended = false;
    this.prevLastHour = -Infinity;
  }

  copyFrom(other: LineTrace, sink: LineSink): void {
    this.step = other.step;
    this.scale = other.scale;
    this.binHours = other.binHours;
    this.sink = sink;
    this.open = other.open;
    this.lastHour = other.lastHour;
    this.lastValue = other.lastValue;
    this.ended = other.ended;
    this.prevLastHour = other.prevLastHour;
  }

  continueInto(sink: LineSink): LineTrace {
    const next = new LineTrace(this.step, this.scale, this.binHours, sink);
    next.open = this.open;
    next.lastHour = this.lastHour;
    next.lastValue = this.lastValue;
    next.ended = this.ended;
    next.prevLastHour = this.prevLastHour;
    return next;
  }

  private start(hour: number, value: number): void {
    this.sink.move(hour, value);
    this.open = true;
    this.lastHour = hour;
    this.lastValue = value;
  }

  private to(hour: number, value: number): void {
    this.sink.line(hour, value);
    this.lastHour = hour;
    this.lastValue = value;
  }

  private point(hour: number, value: number): void {
    if (!this.open) this.start(hour, value);
    else {
      if (this.lastHour === hour && this.lastValue === value) return;
      if (this.step) this.to(hour, this.lastValue);
      this.to(hour, value);
    }
    this.ended = true;
  }

  bucket(b: ChartBucket | null | undefined, window: LineWindow): void {
    if (!b || b.lastHour < window.from || b.firstHour > window.to) return;
    if (this.open && (b.firstBreak || b.firstHour > this.prevLastHour + Math.max(1, this.binHours))) this.open = false;
    const scale = this.scale;
    const at = (hour: number, value: number) => this.point(Math.max(window.from, Math.min(window.to, hour)), value * scale);
    if (this.step) {
      at(b.firstHour, b.firstValue);
      if (b.minValue !== b.firstValue || b.maxValue !== b.firstValue) {
        if (b.minHour <= b.maxHour) {
          if (b.minValue !== b.firstValue) at(b.minHour, b.minValue);
          if (b.maxValue !== b.minValue) at(b.maxHour, b.maxValue);
        } else {
          if (b.maxValue !== b.firstValue) at(b.maxHour, b.maxValue);
          if (b.minValue !== b.maxValue) at(b.minHour, b.minValue);
        }
      }
      if (b.lastHour !== b.firstHour || b.lastValue !== b.firstValue) at(b.lastHour, b.lastValue);
    } else {
      at(b.firstHour, b.firstValue);
      const loEnd = Math.min(b.firstValue, b.lastValue);
      const hiEnd = Math.max(b.firstValue, b.lastValue);
      const hasLower = b.minValue < loEnd;
      const hasHigher = b.maxValue > hiEnd;
      if (hasLower && hasHigher) {
        if (b.minHour <= b.maxHour) {
          at(b.minHour, b.minValue);
          at(b.maxHour, b.maxValue);
        } else {
          at(b.maxHour, b.maxValue);
          at(b.minHour, b.minValue);
        }
      } else if (hasLower) at(b.minHour, b.minValue);
      else if (hasHigher) at(b.maxHour, b.maxValue);
      const endHour = b.lastValueHour >= b.firstHour ? b.lastValueHour : b.lastHour;
      if (endHour > b.firstHour || b.lastValue !== b.firstValue) at(endHour, b.lastValue);
    }
    this.prevLastHour = b.lastHour;
  }

  run(r: Run, window: LineWindow): void {
    if (r.lastHour < window.from || r.firstHour > window.to) return;
    if (r.breakBefore && this.open) this.open = false;
    const x0 = Math.max(r.firstHour, window.from);
    const x1 = Math.min(r.lastHour, window.to);
    const value = r.value * this.scale;
    if (!this.open) {
      this.start(x0, value);
      if (this.step && x1 > x0) this.to(x1, value);
    } else if (this.step) {
      this.to(x0, this.lastValue);
      this.to(x0, value);
      if (x1 > x0) this.to(x1, value);
    } else {
      this.to(x0, value);
    }
    this.ended = true;
  }

  head(hour: number, value: number): void {
    if (!this.ended) {
      this.start(hour, value);
      this.ended = true;
      return;
    }
    if (hour < this.lastHour) {
      this.lastHour = hour;
      this.lastValue = value;
      return;
    }
    if (hour > this.lastHour || value !== this.lastValue) {
      if (this.step) this.to(hour, this.lastValue);
      this.to(hour, value);
    }
  }
}

export interface HistorySubpath {
  coords: Float64Array;
  readonly firstHour: number;
  readonly firstValue: number;
  lastHour: number;
  lastValue: number;
  count: number;
}

export interface ClosedSpan {
  readonly from: number;
  readonly to: number;
  readonly items: number;
}

export interface LineHistory extends ClosedSpan {
  readonly subpaths: readonly HistorySubpath[];
  readonly trace: LineTrace;
}

export interface ClosedExtent extends ClosedSpan {
  readonly low: number;
  readonly high: number;
}

interface Building {
  subpaths: HistorySubpath[];
  trace: LineTrace;
  from: number;
  to: number;
  items: number;
}

interface Extent {
  from: number;
  to: number;
  low: number;
  high: number;
  items: number;
}

const WHOLE: LineWindow = { from: -Infinity, to: Infinity };
const histories = new WeakMap<object, Map<string, Building>>();
const INITIAL_SUBPATH_FLOATS = 64;

function coordSink(state: Building): LineSink {
  return {
    move(hour, value) {
      const coords = new Float64Array(INITIAL_SUBPATH_FLOATS);
      coords[0] = hour;
      coords[1] = value;
      state.subpaths.push({ coords, firstHour: hour, firstValue: value, lastHour: hour, lastValue: value, count: 1 });
    },
    line(hour, value) {
      const sub = state.subpaths[state.subpaths.length - 1]!;
      const offset = sub.count * 2;
      if (offset + 2 > sub.coords.length) {
        const grown = new Float64Array(sub.coords.length * 2);
        grown.set(sub.coords);
        sub.coords = grown;
      }
      sub.coords[offset] = hour;
      sub.coords[offset + 1] = value;
      sub.lastHour = hour;
      sub.lastValue = value;
      sub.count++;
    },
  };
}

function closedSlice(bounded: BoundedSeries) {
  return bounded.runs && bounded.runs.length > 0 ? bounded.runs : bounded.buckets;
}

function variantsOf<T>(cache: WeakMap<object, Map<string, T>>, source: object): Map<string, T> {
  let variants = cache.get(source);
  if (!variants) cache.set(source, variants = new Map());
  return variants;
}

const extents = new WeakMap<object, Map<string, Extent>>();

export function closedExtent(bounded: BoundedSeries, scale: number): ClosedExtent {
  const slice = closedSlice(bounded);
  const useRuns = slice !== bounded.buckets;
  const variants = variantsOf(extents, slice.source);
  const key = `${useRuns ? 'r' : 'b'}${scale}`;
  let state = variants.get(key);
  if (!state || state.items > slice.closedLength) variants.set(key, state = { from: Infinity, to: -Infinity, low: Infinity, high: -Infinity, items: 0 });
  const include = (first: number, last: number, low: number, high: number) => {
    if (first < state.from) state.from = first;
    if (last > state.to) state.to = last;
    if (Number.isFinite(low) && low < state.low) state.low = low;
    if (Number.isFinite(high) && high > state.high) state.high = high;
  };
  for (let i = state.items; i < slice.closedLength; i++) {
    if (useRuns) {
      const r = bounded.runs!.at(i)!;
      include(r.firstHour, r.lastHour, r.value * scale, r.value * scale);
    } else {
      const b = bounded.buckets.at(i);
      if (b) include(b.firstHour, b.lastHour, b.minValue * scale, b.maxValue * scale);
    }
  }
  state.items = slice.closedLength;
  return state;
}

export function lineHistory(bounded: BoundedSeries, step: boolean, scale: number): LineHistory {
  const slice = closedSlice(bounded);
  const useRuns = slice !== bounded.buckets;
  const variants = variantsOf(histories, slice.source);
  const key = `${useRuns ? 'r' : 'b'}${step ? 's' : 'l'}${scale}:${bounded.binHours}`;
  let state = variants.get(key);
  if (!state || state.items > slice.closedLength) {
    state = { subpaths: [], trace: null!, from: Infinity, to: -Infinity, items: 0 };
    state.trace = new LineTrace(step, scale, bounded.binHours, coordSink(state));
    variants.set(key, state);
  }
  for (let i = state.items; i < slice.closedLength; i++) {
    if (useRuns) {
      const r = bounded.runs!.at(i)!;
      state.trace.run(r, WHOLE);
      if (r.firstHour < state.from) state.from = r.firstHour;
      if (r.lastHour > state.to) state.to = r.lastHour;
    } else {
      const b = bounded.buckets.at(i);
      state.trace.bucket(b, WHOLE);
      if (b && b.firstHour < state.from) state.from = b.firstHour;
      if (b && b.lastHour > state.to) state.to = b.lastHour;
    }
  }
  state.items = slice.closedLength;
  return state;
}

export function historyFits(history: ClosedSpan, window: LineWindow): boolean {
  return history.items === 0 || (window.from <= history.from && history.to <= window.to);
}
