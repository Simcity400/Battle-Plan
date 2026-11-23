import type { KitRaster, PathBuilder, PathPaint } from './path';

export type Align = 'left' | 'right' | 'center';
type Baseline = 'top' | 'middle' | 'bottom' | 'alphabetic';
export interface LabelStyle { font: string; fill: string; align: Align; baseline: Baseline; squeeze?: boolean }

export interface Kit {
  begin(width: number, height: number, background?: string): void;
  rect(x: number, y: number, width: number, height: number, fill: string, radius?: number): void;
  rule(x0: number, y0: number, x1: number, y1: number, stroke: string, width?: number): void;
  area(xs: ArrayLike<number>, highs: ArrayLike<number>, lows: ArrayLike<number>, count: number, fill: string, outline?: string): void;
  line(xs: ArrayLike<number>, ys: ArrayLike<number>, count: number, stroke: string, width: number): void;
  label(text: string, x: number, y: number, style: LabelStyle, maxWidth?: number): number;
  measure(text: string, font: string): number;
  icon(key: string, x: number, y: number, width: number, height: number): boolean;
  fade(alpha: number): void;
  dashed(pattern: readonly number[]): void;
  desaturate(): void;
  clip(x: number, y: number, width: number, height: number): void;
  newPath(): PathBuilder;
  clipPath(path: PathBuilder): void;
  path(path: PathBuilder, paint: PathPaint): void;
  erase(x: number, y: number, width: number, height: number): void;
  raster(raster: KitRaster, sx: number, sy: number, sw: number, sh: number, dx: number, dy: number, dw: number, dh: number): void;
  readonly pixelRatio: number;
  save(): void;
  restore(): void;
  place?(axes: Placement): void;
  when?(span: readonly [number, number] | null): void;
  pulse?(x: number, y: number, size: number, fill: string): void;
}

export type Placement = 'pixel' | 'x' | 'y' | 'xy';

export interface DrawRegion {
  x: number; y: number; width: number; height: number;
  clip: [left: number, top: number, right: number, bottom: number];
}
