import {
  VALUE_LABEL_GAP,
  clipToPlot,
  drawAxes,
  drawPlayhead,
  valueLabelWidth,
} from './draw.ts';
import { plotFrame, type Padding, type PlotFrame } from './frame.ts';
import { canvasMeasureText, type Canvas2D, type MeasureText } from './textWidth.ts';
import { CHART_LAYOUT, type ChartTokens } from './tokens.ts';
import { formatAxisValue } from './valueAxis.ts';
import { playbackDomain, resolveX, resolveY, shownUntil, type AxisSettings, type PlaybackMode, type Range, type ViewState } from './view.ts';

export const TIME_CHART_DEFAULT_PADDING: Padding = { left: 44, right: CHART_LAYOUT.right, top: 12, bottom: 24 };
export const TIME_CHART_LABEL_MARGIN = 4;
export const TIME_CHART_MIN_SPAN = 14 * 24;

export type AxisValueFormatter = (value: number, ticks: readonly number[]) => string;

export interface ChartLayer<C extends Canvas2D = Canvas2D> {
  draw(ctx: C, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void;
  extent?(x: Range): Range | null;
}

export interface TimeChartDomainInput {
  readonly extent: Range;
  readonly playhead: number | null;
  readonly playback: PlaybackMode;
  readonly windowHours?: number;
  readonly paused: boolean;
  readonly view: ViewState;
  readonly axes: AxisSettings;
  readonly branchStart: number;
}

export function resolveTimeChartTargetX(input: TimeChartDomainInput): Range {
  const playback = input.playhead == null
    ? input.extent
    : playbackDomain(input.playback, input.extent, input.playhead, input.windowHours ?? 365 * 24, TIME_CHART_MIN_SPAN, input.paused);
  return resolveX(input.view, input.axes, input.branchStart, playback);
}

export function resolveTimeChartTargetY(
  layers: readonly Pick<ChartLayer, 'extent'>[],
  x: Range,
  view: ViewState,
  axes: AxisSettings,
): { range: Range; log: boolean } {
  let data: [number, number] | null = null;
  for (const layer of layers) {
    const extent = layer.extent?.(x);
    if (extent) data = data ? [Math.min(data[0], extent[0]), Math.max(data[1], extent[1])] : [extent[0], extent[1]];
  }
  return resolveY(view, axes, data);
}

export interface FitTimeChartFrameInput {
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly padding: Padding;
  readonly x: Range;
  readonly y: Range;
  readonly log: boolean;
  readonly tokens: ChartTokens;
  readonly formatValue?: AxisValueFormatter;
  readonly measure: MeasureText;
}

export function fitTimeChartFrame(input: FitTimeChartFrameInput): PlotFrame {
  const {
    layoutWidth,
    layoutHeight,
    padding,
    x,
    y,
    log,
    tokens,
    formatValue,
    measure,
  } = input;
  let frame = plotFrame(layoutWidth, layoutHeight, padding, x, y, log);
  const formatter = formatValue ?? formatAxisValue;
  const needed = Math.ceil(valueLabelWidth(null, frame, tokens, formatter, measure)) + VALUE_LABEL_GAP + TIME_CHART_LABEL_MARGIN;
  if (needed > padding.left) {
    frame = plotFrame(layoutWidth, layoutHeight, { ...padding, left: needed }, x, y, log);
  }
  return frame;
}

export interface TimeChartScene<C extends Canvas2D = Canvas2D> {
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly scaleX: number;
  readonly scaleY: number;
  readonly layers: readonly ChartLayer<C>[];
  readonly playhead: number | null;
  readonly playback: PlaybackMode;
  readonly afterPlayhead?: boolean;
  readonly until?: number | null;
  readonly formatValue?: AxisValueFormatter;
  readonly overlay?: (ctx: C, frame: PlotFrame, tokens: ChartTokens) => void;
}

export function drawTimeChartScene<C extends Canvas2D>(
  ctx: C,
  frame: PlotFrame,
  tokens: ChartTokens,
  scene: TimeChartScene<C>,
  measure?: MeasureText,
): void {
  const measureText = measure ?? canvasMeasureText(ctx);
  const formatter = scene.formatValue ?? formatAxisValue;
  const until = scene.until !== undefined
    ? scene.until
    : shownUntil(scene.playhead, scene.playback, scene.afterPlayhead === true);
  ctx.setTransform(scene.scaleX, 0, 0, scene.scaleY, 0, 0);
  ctx.clearRect(0, 0, scene.layoutWidth, scene.layoutHeight);
  drawAxes(ctx, frame, tokens, formatter, measureText);
  for (const layer of scene.layers) {
    ctx.save();
    clipToPlot(ctx, frame);
    layer.draw(ctx, frame, tokens, { playhead: until });
    ctx.restore();
  }
  if (scene.playhead != null && scene.playback === 'cursor') drawPlayhead(ctx, frame, tokens, scene.playhead);
  scene.overlay?.(ctx, frame, tokens);
}

