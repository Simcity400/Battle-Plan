import type { KitRaster, PathBuilder, PathPaint } from './path';

export type Align = 'left' | 'right' | 'center';
type Baseline = 'top' | 'middle' | 'bottom' | 'alphabetic';
/** `squeeze`: text wider than the label's room is squeezed to it, as a 2D context's `maxWidth`, not ellipsised. */
export interface LabelStyle { font: string; fill: string; align: Align; baseline: Baseline; squeeze?: boolean }

/** The primitives every drawn widget is made of, in layout pixels. */
export interface Kit {
  /** Starts a frame of `width` x `height`, filled with `background` (the widget's surface, so the drawing covers its
   * DOM view the frame it is shown), or cleared without one. */
  begin(width: number, height: number, background?: string): void;
  rect(x: number, y: number, width: number, height: number, fill: string, radius?: number): void;
  /** A hairline from one point to another, snapped to the pixel grid. */
  rule(x0: number, y0: number, x1: number, y1: number, stroke: string, width?: number): void;
  /** A filled band between `highs` and `lows` over `xs` (`count` points), outlined in `outline` when given. */
  area(xs: ArrayLike<number>, highs: ArrayLike<number>, lows: ArrayLike<number>, count: number, fill: string, outline?: string): void;
  /** A polyline through `count` points; a NaN `y` breaks it. */
  line(xs: ArrayLike<number>, ys: ArrayLike<number>, count: number, stroke: string, width: number): void;
  /** Text at a point, ellipsised (or squeezed, `style.squeeze`) to `maxWidth` when given. Returns the width drawn. */
  label(text: string, x: number, y: number, style: LabelStyle, maxWidth?: number): number;
  measure(text: string, font: string): number;
  /** An image the kit holds by `key` (flags); false, drawing nothing, until it is held. */
  icon(key: string, x: number, y: number, width: number, height: number): boolean;
  /** Multiplies the opacity of what is drawn until `restore`. */
  fade(alpha: number): void;
  /** Dashes strokes until `restore` (an approximate stretch); a path's own dash wins. */
  dashed(pattern: readonly number[]): void;
  /** Draws in grey until `restore`, as CSS `filter: grayscale(1)` (a muted row). */
  desaturate(): void;
  clip(x: number, y: number, width: number, height: number): void;
  /** Begins a path, painted or clipped before the next is begun. */
  newPath(): PathBuilder;
  /** Limits drawing to inside `path` until `restore`. */
  clipPath(path: PathBuilder): void;
  /** A path, filled and/or stroked. */
  path(path: PathBuilder, paint: PathPaint): void;
  /** Clears a rectangle to transparent (a gutter redrawn over what was there). */
  erase(x: number, y: number, width: number, height: number): void;
  /** Part of a raster's pixels (`sx`, `sy`, `sw`, `sh`, in its pixels) shown over a rectangle (layout pixels). */
  raster(raster: KitRaster, sx: number, sy: number, sw: number, sh: number, dx: number, dy: number, dw: number, dh: number): void;
  /** Device pixels a layout pixel covers here, horizontally: what exact rasters are made at. */
  readonly pixelRatio: number;
  save(): void;
  restore(): void;
  /** What the primitives drawn next (until `restore`) are placed by in a plot: the time axis (`x`), the value axis
   * (`y`), both, or neither. A kit drawing every frame ignores it; a retained one moves them with the plot's view. */
  place?(axes: Placement): void;
  /** For a retained plot, reveal subsequent primitives when its clock reaches the given pixel range. */
  when?(span: readonly [number, number] | null): void;
  /** A live dot's halo (a dot `size` wide at x, y), pulsing by the kit's own clock; a kit drawing every frame has none and
   * draws the halo at the frame's time itself. */
  pulse?(x: number, y: number, size: number, fill: string): void;
}

export type Placement = 'pixel' | 'x' | 'y' | 'xy';

/** The playback clock as the worker reads it: the position at `at` (epoch ms), moving at `speed` while playing. */
export interface ClockAnchor {
  position: number; at: number; playing: boolean; speed: number; lastHour: number;
  /** Host generation for dropping notices queued before the latest anchor. */
  generation?: number;
  /** The clock's seeks so far: a new one means steps built ahead on the path it left are let go. */
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
  | { type: 'detach'; id: number }
  | { type: 'pointer'; id: number; owner: number; point: [number, number] | null }
  | { type: 'race'; id: number; owner: number; model: import('./arsenal').ArsenalModel & { view: keyof typeof import('./panels/views').raceViews }; canvas?: OffscreenCanvas }
  | { type: 'closeRace'; id: number }
  | { type: 'racePointer'; id: number; owner: number; point?: { x: number; y: number } }
  | { type: 'inspectRace'; id: number; owner: number; request: number; target?: import('./arsenal').RaceTarget }
  | { type: 'inspect'; id: number; request: number }
  | { type: 'options'; rate: number; measured: boolean; cap: number };

export type FromWorker = { type: 'displayRate'; hz: number } | import('./notifications').Notice
  | { type: 'audit'; records: import('./notifications').DrawSample[]; done: boolean }
  | { type: 'raceHover'; id: number; owner: number; seek: number; item?: import('../charts/race/race').RaceItem }
  | { type: 'raceInspection'; id: number; owner: number; request: number; seek: number; picture?: import('./arsenal').RaceInspection }
  | { type: 'racePresent'; id: number; owner: number; seek: number; generation: number; head: number; hour: number | null; at: number; rows?: readonly { row: string; key: string; value: string }[] }
  | { type: 'inspection'; request: number; reading?: import('./followed').PageReading }
  | { type: 'structure'; updates: import('./followed').PageReading[] }
  | { type: 'structureLead'; updates: import('./followed').PageReading[] }
  | { type: 'resetStructures'; route: number; seek: number; from: number }
  | { type: 'followFailure'; reason: string }
  | { type: 'failed'; page: number; reason: string };
