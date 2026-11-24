import type { PlotFrame } from './frame.ts';
import type { KeyTarget } from './hover.ts';
import type { Canvas2D } from './textWidth.ts';
import type { Range } from './view.ts';
import type { ChartTokens } from './tokens.ts';

export interface ChartStrip {
  height(chartHeight: number): number;
  draw(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void;
  describe?(point: { x: number; y: number }, frame: PlotFrame, until: number | null): number | null;
  keyTargets?(view: Range, until: number | null): readonly KeyTarget[];
}
