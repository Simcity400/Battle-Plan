/** A path as drawing code builds it, from `Kit.newPath()`: its moves, lines, curves, arcs and rectangles, in layout pixels.
 * A backend builds it its own way (a 2D canvas straight into its context's path, a GPU by recording it), so a path is
 * painted or clipped (`Kit.path`, `Kit.clipPath`) before the next is begun. */
export interface PathBuilder {
  moveTo(x: number, y: number): this;
  lineTo(x: number, y: number): this;
  bezierCurveTo(x1: number, y1: number, x2: number, y2: number, x: number, y: number): this;
  arc(x: number, y: number, radius: number, start: number, end: number): this;
  /** A corner of `radius` from the current point toward (x1, y1) and on toward (x2, y2), as a 2D context's. */
  arcTo(x1: number, y1: number, x2: number, y2: number, radius: number): this;
  rect(x: number, y: number, width: number, height: number): this;
  closePath(): this;
  readonly empty: boolean;
}

/** How a path is painted: filled, stroked (with its width, dash, joins and caps) or both, at an opacity. */
export interface PathPaint {
  fill?: string;
  stroke?: string;
  width?: number;
  dash?: readonly number[];
  join?: 'round' | 'miter' | 'bevel';
  cap?: 'butt' | 'round' | 'square';
  alpha?: number;
}

/** Pixels a backend shows (a chart's exact share columns): RGBA rows, `dirty` the columns changed since it last showed
 * them (null: none), so it uploads only those. */
export interface KitRaster { width: number; height: number; pixels: Uint8ClampedArray<ArrayBuffer>; dirty: [number, number] | null }
