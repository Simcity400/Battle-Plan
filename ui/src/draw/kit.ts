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

export interface ClockAnchor {
  position: number; at: number; playing: boolean; speed: number; lastHour: number;
  generation?: number;
  seek: number;
  forwardSeek?: boolean;
}

export function anchorHour(anchor: ClockAnchor, now: number): number {
  if (!anchor.playing) return anchor.position;
  return Math.min(anchor.lastHour, anchor.position + Math.max(0, now - anchor.at) / 1000 * anchor.speed);
}

export interface DrawRegion {
  x: number; y: number; width: number; height: number;
  clip: [left: number, top: number, right: number, bottom: number];
}


export type ToWorker =
  | { type: 'audit'; start: boolean; limit?: number }
  | { type: 'port'; port: MessagePort }
  | { type: 'add'; id: number; branch: string; product?: string }
  | { type: 'remove'; id: number }
  | { type: 'seek'; branch: string; seek: number; retainThrough?: number }
  | { type: 'bind'; branch: string; route: number; seek: number }
  | { type: 'clock'; anchor: ClockAnchor }
  | { type: 'surface'; id: number; owner: number; width: number; height: number; dpr: number; canvas?: OffscreenCanvas; model: import('./panels/view').PanelModel }
  | { type: 'table'; id: number; owner: number; width: number; height: number; dpr: number; canvas?: OffscreenCanvas; model: import('./tables/surface').TableSurfaceModel }
  | { type: 'dateFace'; id: number; width: number; height: number; dpr: number; canvas?: OffscreenCanvas; face: import('./dateFace').DateFaceModel }
  | { type: 'detach'; id: number }
  | { type: 'pointer'; id: number; owner: number; point: [number, number] | null }
  | { type: 'tablePointer'; id: number; owner: number; point: [number, number] | null; scrollLeft?: number }
  | { type: 'tableScroll'; id: number; owner: number; scrollLeft: number; scrollTop?: number }
  | { type: 'tableRead'; id: number; owner: number; request: number; keys?: readonly string[]; rows?: { first: number; count: number }; detail?: string }
  | { type: 'race'; id: number; owner: number; model: import('./arsenal').ArsenalModel & { view: keyof typeof import('./panels/views').raceViews }; canvas?: OffscreenCanvas }
  | { type: 'closeRace'; id: number }
  | { type: 'racePointer'; id: number; owner: number; point?: { x: number; y: number } }
  | { type: 'inspectRace'; id: number; owner: number; request: number; target?: import('./arsenal').RaceTarget }
  | { type: 'inspect'; id: number; request: number }
  | { type: 'options'; rate: number; measured: boolean; cap: number; lane: number };

export type FromWorker = { type: 'displayRate'; hz: number } | import('./notifications').Notice
  | { type: 'audit'; records: import('./notifications').DrawSample[]; done: boolean }
  | { type: 'raceHover'; id: number; owner: number; seek: number; item?: import('../charts/race/race').RaceItem }
  | { type: 'raceInspection'; id: number; owner: number; request: number; seek: number; picture?: import('./arsenal').RaceInspection }
  | { type: 'racePresent'; id: number; owner: number; seek: number; generation: number; head: number; hour: number | null; at: number; shown?: number; rows?: readonly { row: string; key: string; value: string }[] }
  | { type: 'inspection'; request: number; reading?: import('./followed').PageReading }
  | { type: 'tableRead'; id: number; owner: number; request: number; reading: import('./tables/surface').TableReading | null }
  | { type: 'tableRows'; id: number; owner: number; count: number; empty?: string }
  | { type: 'tableColumns'; id: number; owner: number; widths: Record<string, number> }
  | { type: 'structure'; updates: import('./followed').PageReading[] }
  | { type: 'structureLead'; updates: import('./followed').PageReading[] }
  | { type: 'resetStructures'; route: number; seek: number; from: number }
  | { type: 'followFailure'; reason: string }
  | { type: 'failed'; page: number; reason: string };
