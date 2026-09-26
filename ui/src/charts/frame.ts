import type { Range } from './view.ts';

/** Where the plot sits inside the canvas, and how hours and values map to pixels. */
export interface Padding { left: number; right: number; top: number; bottom: number }
export interface PlotFrame {
  width: number; height: number; padding: Padding;
  plotLeft: number; plotTop: number; plotWidth: number; plotHeight: number;
  x: Range; y: Range; log: boolean;
  toX(hour: number): number;
  toY(value: number): number;
  hourAt(px: number): number;
  valueAt(py: number): number;
}

const MIN_PLOT = 8;

export function plotFrame(width: number, height: number, padding: Padding, x: Range, y: Range, log: boolean): PlotFrame {
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
    toX: hour => plotLeft + (hour - x[0]) / span * plotWidth,
    toY: value => plotTop + plotHeight - (scale(value) - low) / depth * plotHeight,
    hourAt: px => x[0] + (px - plotLeft) / plotWidth * span,
    valueAt: py => {
      const fraction = (plotTop + plotHeight - py) / plotHeight;
      return logged ? 10 ** (low + fraction * depth) : low + fraction * depth;
    },
  };
}

export type HitZone = 'x-axis' | 'y-axis' | 'plot' | null;

/** Which part of the chart a point is over: the date axis below the plot, the value
 * axis to its left, or the plot itself. */
export function hitZone(point: { x: number; y: number }, frame: PlotFrame): HitZone {
  const right = frame.plotLeft + frame.plotWidth;
  const bottom = frame.plotTop + frame.plotHeight;
  if (point.y > bottom && point.y <= frame.height && point.x >= frame.plotLeft && point.x <= right) return 'x-axis';
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

/** A bar's rectangle: thin (at most `maxThickness`), square at the baseline. */
export function barRect(center: number, thickness: number, from: number, to: number, maxThickness = 20): { x: number; y: number; width: number; height: number } {
  const size = Math.min(maxThickness, Math.max(1, thickness));
  return { x: center - size / 2, y: Math.min(from, to), width: size, height: Math.abs(to - from) };
}
