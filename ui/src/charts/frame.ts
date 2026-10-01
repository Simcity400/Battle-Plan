import type { Range } from './view.ts';

/** Where the plot sits inside the canvas, and how hours and values map to pixels. */
export interface Padding { left: number; right: number; top: number; bottom: number }
/** A band under the plot on the same time axis (such as daily bars), above the dates. */
export interface StripBox { top: number; height: number }
export interface PlotFrame {
  width: number; height: number; padding: Padding;
  plotLeft: number; plotTop: number; plotWidth: number; plotHeight: number;
  /** The strip under the plot, when the chart has one; the bottom padding holds it. */
  strip: StripBox | null;
  x: Range; y: Range; log: boolean;
  toX(hour: number): number;
  toY(value: number): number;
  hourAt(px: number): number;
  valueAt(py: number): number;
}

const MIN_PLOT = 8;
/** Space between the plot and a strip under it. */
export const STRIP_GAP = 10;

/** `strip` is the height of a band under the plot, inside the bottom padding. */
export function plotFrame(width: number, height: number, padding: Padding, x: Range, y: Range, log: boolean, strip = 0): PlotFrame {
  const plotLeft = padding.left;
  const plotTop = padding.top;
  const plotWidth = Math.max(MIN_PLOT, width - padding.left - padding.right);
  const plotHeight = Math.max(MIN_PLOT, height - padding.top - padding.bottom);
  const span = Math.max(1e-9, x[1] - x[0]);
  const logged = log && y[0] > 0;
  const low = logged ? Math.log10(y[0]) : y[0];
  const high = logged ? Math.log10(y[1]) : y[1];
  const depth = Math.max(1e-12, high - low);
  const scale = (value: number) => logged ? Math.log10(Math.max(value, y[0] * 1e-3)) : value;
  return {
    width, height, padding, plotLeft, plotTop, plotWidth, plotHeight, x, y, log: logged,
    strip: strip > 0 ? { top: plotTop + plotHeight + STRIP_GAP, height: strip } : null,
    toX: hour => plotLeft + (hour - x[0]) / span * plotWidth,
    toY: value => plotTop + plotHeight - (scale(value) - low) / depth * plotHeight,
    hourAt: px => x[0] + (px - plotLeft) / plotWidth * span,
    valueAt: py => {
      const fraction = (plotTop + plotHeight - py) / plotHeight;
      return logged ? 10 ** (low + fraction * depth) : low + fraction * depth;
    },
  };
}

export type HitZone = 'x-axis' | 'y-axis' | 'plot' | 'strip' | null;

/** The bottom of what the time axis spans: the strip's, else the plot's. */
export function chartBottom(frame: PlotFrame): number {
  return frame.strip ? frame.strip.top + frame.strip.height : frame.plotTop + frame.plotHeight;
}

/** Which part of the chart a point is over: the date axis below the plot (and any
 * strip), the value axis to its left, the plot itself, or the strip (with the gap
 * above it). */
export function hitZone(point: { x: number; y: number }, frame: PlotFrame): HitZone {
  const right = frame.plotLeft + frame.plotWidth;
  const bottom = frame.plotTop + frame.plotHeight;
  const inTime = point.x >= frame.plotLeft && point.x <= right;
  if (frame.strip && point.y > bottom && point.y <= chartBottom(frame) && inTime) return 'strip';
  if (point.y > chartBottom(frame) && point.y <= frame.height && inTime) return 'x-axis';
  if (point.x < frame.plotLeft && point.x >= 0 && point.y >= frame.plotTop && point.y <= bottom) return 'y-axis';
  if (point.x >= frame.plotLeft && point.x <= right && point.y >= frame.plotTop && point.y <= bottom) return 'plot';
  return null;
}

/** How far along an axis a point is: 0 at its start (left or bottom), 1 at its end. */
export function axisFraction(point: { x: number; y: number }, frame: PlotFrame, axis: 'x' | 'y'): number {
  return axis === 'x' ? (point.x - frame.plotLeft) / frame.plotWidth : (frame.plotTop + frame.plotHeight - point.y) / frame.plotHeight;
}

/** Zoom steps for a wheel event: proportional to how far it scrolled, so a trackpad's
 * small deltas zoom a little and a mouse notch zooms one step. Sideways-only
 * scrolling is not a zoom. */
export function wheelSteps(deltaY: number, deltaMode: number): number {
  const pixels = deltaY * (deltaMode === 1 ? 33 : deltaMode === 2 ? 400 : 1);
  return Math.max(-3, Math.min(3, pixels / 100));
}

/** The hour a pointer over the plot may show: never past the playhead, and none at all
 * when the whole visible range is after it. */
export function pointerHour(hourAtPointer: number, x: readonly [number, number], playhead: number | null): number | null {
  const last = Math.min(x[1], playhead ?? x[1]);
  if (last < x[0]) return null;
  return Math.max(x[0], Math.min(last, hourAtPointer));
}

/** Hours the keyboard hover steps by when a chart names no stops: a day. */
const KEY_STEP = 24;

/** The stops the keyboard hover may land on in a view from `first` to `last`: the
 * stops inside it, and the view's start itself when a stop before the view carries
 * its value into it. A date range or zoom that starts between two daily reads (reads
 * at 100 and 124, a view of 110 to 120) so still reaches the value read at 100, as
 * the pointer does; a zoom starting partway through an hour (23.2) stops at its
 * start, which hover shows as that hour (23), as the pointer at the edge does, even
 * when the next stop is the next whole hour. Pass every stop, not only those in view. */
export function keyStops(stops: readonly number[], first: number, last: number): number[] {
  const inside = stops.filter(hour => hour >= first && hour <= last);
  const carried = first <= last && (inside[0] ?? Infinity) > first && stops.some(hour => hour < first);
  return carried ? [first, ...inside] : inside;
}

/** Where a key moves keyboard hover: through `stops` (ascending hours, such as each
 * reading, and any value carried into the view: `keyStops`), else a day at a time,
 * only over hours the pointer could show. The first arrow press starts at `current`,
 * else the latest hour shown; Home and End go to the first and last. Null when
 * nothing in view can be shown. */
export function keyedHour(key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', current: number | null, x: readonly [number, number],
  playhead: number | null, stops?: readonly number[]): number | null {
  const first = x[0];
  const last = Math.min(x[1], playhead ?? x[1]);
  if (last < first) return null;
  if (stops) {
    const shown = keyStops(stops, first, last);
    if (!shown.length) return null;
    if (key === 'Home') return shown[0];
    if (key === 'End' || current == null) return shown.at(-1)!;
    const before = shown.filter(hour => hour < current);
    const after = shown.filter(hour => hour > current);
    // At either end the hover stays on the nearest stop shown.
    if (key === 'ArrowRight') return after[0] ?? shown.filter(hour => hour <= current).at(-1)!;
    return before.at(-1) ?? shown.filter(hour => hour >= current)[0];
  }
  const clamp = (hour: number) => Math.floor(Math.max(first, Math.min(last, hour)));
  if (key === 'Home') return clamp(first);
  if (key === 'End' || current == null) return clamp(last);
  return clamp(current + (key === 'ArrowRight' ? KEY_STEP : -KEY_STEP));
}
