import type { Range } from './view.ts';

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

export function hitZone(point: { x: number; y: number }, frame: PlotFrame): HitZone {
  const right = frame.plotLeft + frame.plotWidth;
  const bottom = frame.plotTop + frame.plotHeight;
  const inTime = point.x >= frame.plotLeft && point.x <= right;
  if (point.y > bottom && point.y <= frame.height && inTime) return 'x-axis';
  if (point.x < frame.plotLeft && point.x >= 0 && point.y >= frame.plotTop && point.y <= bottom) return 'y-axis';
  if (point.x >= frame.plotLeft && point.x <= right && point.y >= frame.plotTop && point.y <= bottom) return 'plot';
  return null;
}

export function axisFraction(point: { x: number; y: number }, frame: PlotFrame, axis: 'x' | 'y'): number {
  return axis === 'x' ? (point.x - frame.plotLeft) / frame.plotWidth : (frame.plotTop + frame.plotHeight - point.y) / frame.plotHeight;
}

export function wheelSteps(deltaY: number, deltaMode: number): number {
  const pixels = deltaY * (deltaMode === 1 ? 33 : deltaMode === 2 ? 400 : 1);
  return Math.max(-3, Math.min(3, pixels / 100));
}

export function pointerHour(hourAtPointer: number, x: readonly [number, number], playhead: number | null): number | null {
  const last = Math.min(x[1], playhead ?? x[1]);
  if (last < x[0]) return null;
  return Math.max(x[0], Math.min(last, hourAtPointer));
}

const KEY_STEP = 24;

export function keyStops(stops: readonly number[], first: number, last: number): number[] {
  const inside = stops.filter(hour => hour >= first && hour <= last);
  const carried = first <= last && (inside[0] ?? Infinity) > first && stops.some(hour => hour < first);
  return carried ? [first, ...inside] : inside;
}

export function keyedStopHour(key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', current: number | null, shown: readonly number[]): number | null {
  if (!shown.length) return null;
  if (key === 'Home') return shown[0];
  if (key === 'End' || current == null) return shown.at(-1)!;
  if (key === 'ArrowRight') {
    for (let i = 0; i < shown.length; i++) if (shown[i] > current) return shown[i];
    for (let i = shown.length - 1; i >= 0; i--) if (shown[i] <= current) return shown[i];
    return shown[0];
  }
  for (let i = shown.length - 1; i >= 0; i--) if (shown[i] < current) return shown[i];
  for (let i = 0; i < shown.length; i++) if (shown[i] >= current) return shown[i];
  return shown.at(-1)!;
}

export function keyedHour(key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', current: number | null, x: readonly [number, number],
  playhead: number | null, stops?: readonly number[]): number | null {
  const first = x[0];
  const last = Math.min(x[1], playhead ?? x[1]);
  if (last < first) return null;
  if (stops) return keyedStopHour(key, current, keyStops(stops, first, last));
  const clamp = (hour: number) => Math.floor(Math.max(first, Math.min(last, hour)));
  if (key === 'Home') return clamp(first);
  if (key === 'End' || current == null) return clamp(last);
  return clamp(current + (key === 'ArrowRight' ? KEY_STEP : -KEY_STEP));
}
