export interface PathBuilder {
  moveTo(x: number, y: number): this;
  lineTo(x: number, y: number): this;
  bezierCurveTo(x1: number, y1: number, x2: number, y2: number, x: number, y: number): this;
  arc(x: number, y: number, radius: number, start: number, end: number): this;
  arcTo(x1: number, y1: number, x2: number, y2: number, radius: number): this;
  rect(x: number, y: number, width: number, height: number): this;
  closePath(): this;
  readonly empty: boolean;
}

export interface PathPaint {
  fill?: string;
  stroke?: string;
  width?: number;
  dash?: readonly number[];
  join?: 'round' | 'miter' | 'bevel';
  cap?: 'butt' | 'round' | 'square';
  alpha?: number;
}

export interface KitRaster { width: number; height: number; pixels: Uint8ClampedArray<ArrayBuffer>; dirty: [number, number] | null }
