import { BADGE_STRIP, drawBadges, type EventBadge } from './badges.ts';
import {
  VALUE_LABEL_GAP,
  clipToPlot,
  drawAxes,
  drawPlayhead,
  placeEndLabels,
  valueLabelWidth,
  type LineEnd,
} from './draw.ts';
import { plotFrame, STRIP_GAP, type Padding, type PlotFrame } from './frame.ts';
import { clipGaps, drawGaps, type LostSpan } from './gaps.ts';
import { drawMarkSummary, markSummaryMatches, type MarkSummary } from './markSummary.ts';
import type { ChartStrip } from './strip.ts';
import { canvasMeasureText, type Canvas2D, type MeasureText } from './textWidth.ts';
import { CHART_LAYOUT, type ChartTokens } from './tokens.ts';
import { formatAxisPercent, formatAxisValue } from './valueAxis.ts';
import { playbackDomain, resolveX, resolveY, shownUntil, type AxisSettings, type PlaybackMode, type Range, type ViewState } from './view.ts';

export const TIME_CHART_DEFAULT_PADDING: Padding = { left: 44, right: CHART_LAYOUT.right, top: 12, bottom: 24 };
export const TIME_CHART_LABEL_MARGIN = 4;
export const TIME_CHART_MIN_SPAN = 14 * 24;

export type ChartAxisFormatterId = 'axisValue' | 'powerAxis' | 'percent';
export type ChartFitId = 'none' | 'stabilityZoom';
export type AxisValueFormatter = (value: number, ticks: readonly number[]) => string;
export type ValueFit = (shown: Range | null) => Range | null;

export const powerAxisFormat: AxisValueFormatter = value =>
  Math.abs(value) >= 1000 ? `${Math.round(value / 100) / 10}k` : String(Math.round(value));

export const stabilityZoomFit: ValueFit = shown => {
  if (!shown) return null;
  const low = shown[0] * 100;
  const high = shown[1] * 100;
  const room = Math.max(1, (high - low) * 0.1);
  return [Math.floor(low - room) / 100, Math.ceil(high + room) / 100];
};

export function normalizeStripHeight(height?: number): number {
  return height ? Math.max(0, Math.round(height)) : 0;
}

const FORMATTERS: Record<ChartAxisFormatterId, AxisValueFormatter> = {
  axisValue: formatAxisValue,
  powerAxis: powerAxisFormat,
  percent: formatAxisPercent,
};

const FITS: Record<ChartFitId, ValueFit | undefined> = {
  none: undefined,
  stabilityZoom: stabilityZoomFit,
};

export function resolveAxisFormatter(format?: ChartAxisFormatterId | AxisValueFormatter): AxisValueFormatter {
  if (typeof format === 'function') return format;
  return format ? FORMATTERS[format] : formatAxisValue;
}

export function resolveChartFit(fit?: ChartFitId | ValueFit): ValueFit | undefined {
  if (typeof fit === 'function') return fit;
  return fit ? FITS[fit] : undefined;
}

export interface ChartLayer<C extends Canvas2D = Canvas2D> {
  draw(ctx: C, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void;
  extent?(x: Range): Range | null;
  gaps?: readonly LostSpan[];
}

export interface TimeChartPaddingInput {
  readonly layoutWidth: number;
  readonly tokens: ChartTokens;
  readonly stripHeight: number;
  readonly hasBadges: boolean;
  readonly hasBadgeSummary: boolean;
  readonly padding?: Partial<Padding>;
  readonly rowLabelWidth?: number;
}

export function resolveTimeChartPadding(input: TimeChartPaddingInput): Padding {
  const { layoutWidth, tokens, hasBadges, hasBadgeSummary, padding: custom, rowLabelWidth } = input;
  const stripHeight = normalizeStripHeight(input.stripHeight);
  const padding: Padding = {
    ...TIME_CHART_DEFAULT_PADDING,
    ...(hasBadges || hasBadgeSummary ? { top: BADGE_STRIP } : {}),
    ...custom,
  };
  if (rowLabelWidth !== undefined) {
    padding.left = Math.min(layoutWidth * 0.45, rowLabelWidth);
    padding.top = custom?.top ?? tokens.fontSize.small * 2 + 4;
    padding.bottom = custom?.bottom ?? 8;
  }
  if (stripHeight) padding.bottom += stripHeight + STRIP_GAP;
  return padding;
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
  fitValues?: ChartFitId | ValueFit,
): { range: Range; log: boolean } {
  let data: [number, number] | null = null;
  for (const layer of layers) {
    const extent = layer.extent?.(x);
    if (extent) data = data ? [Math.min(data[0], extent[0]), Math.max(data[1], extent[1])] : [extent[0], extent[1]];
  }
  const fit = resolveChartFit(fitValues);
  const fitted = fit?.(data) ?? null;
  return resolveY(view, fitted ? { ...axes, yMin: axes.yMin ?? fitted[0], yMax: axes.yMax ?? fitted[1] } : axes, data);
}

export interface FitTimeChartFrameInput {
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly padding: Padding;
  readonly x: Range;
  readonly y: Range;
  readonly log: boolean;
  readonly stripHeight: number;
  readonly hasRows: boolean;
  readonly hasExplicitLeftPadding: boolean;
  readonly valueTicks?: readonly number[];
  readonly tokens: ChartTokens;
  readonly formatValue?: ChartAxisFormatterId | AxisValueFormatter;
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
    stripHeight,
    hasRows,
    hasExplicitLeftPadding,
    valueTicks,
    tokens,
    formatValue,
    measure,
  } = input;
  const normalizedStrip = normalizeStripHeight(stripHeight);
  let frame = plotFrame(layoutWidth, layoutHeight, padding, x, y, log, normalizedStrip, hasRows);
  frame.valueTicks = valueTicks;
  if (!hasRows && !hasExplicitLeftPadding) {
    const formatter = resolveAxisFormatter(formatValue);
    const needed = Math.ceil(valueLabelWidth(null, frame, tokens, formatter, measure)) + VALUE_LABEL_GAP + TIME_CHART_LABEL_MARGIN;
    if (needed > padding.left) {
      frame = plotFrame(layoutWidth, layoutHeight, { ...padding, left: needed }, x, y, log, normalizedStrip, hasRows);
      frame.valueTicks = valueTicks;
    }
  }
  return frame;
}

export interface ResolveTimeChartFrameInput extends TimeChartDomainInput {
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly tokens: ChartTokens;
  readonly layers: readonly Pick<ChartLayer, 'extent'>[];
  readonly fitValues?: ChartFitId | ValueFit;
  readonly formatValue?: ChartAxisFormatterId | AxisValueFormatter;
  readonly valueTicks?: readonly number[];
  readonly stripHeight?: number;
  readonly hasBadges?: boolean;
  readonly hasBadgeSummary?: boolean;
  readonly padding?: Partial<Padding>;
  readonly rowLabelWidth?: number;
  readonly measure: MeasureText;
}

export function resolveTimeChartFrame(input: ResolveTimeChartFrameInput): {
  readonly frame: PlotFrame;
  readonly targetX: Range;
  readonly targetY: { readonly range: Range; readonly log: boolean };
  readonly padding: Padding;
} {
  const stripHeight = normalizeStripHeight(input.stripHeight);
  const padding = resolveTimeChartPadding({
    layoutWidth: input.layoutWidth,
    tokens: input.tokens,
    stripHeight,
    hasBadges: input.hasBadges === true,
    hasBadgeSummary: input.hasBadgeSummary === true,
    padding: input.padding,
    rowLabelWidth: input.rowLabelWidth,
  });
  const targetX = resolveTimeChartTargetX(input);
  const targetY = resolveTimeChartTargetY(input.layers, targetX, input.view, input.axes, input.fitValues);
  const frame = fitTimeChartFrame({
    layoutWidth: input.layoutWidth,
    layoutHeight: input.layoutHeight,
    padding,
    x: targetX,
    y: targetY.range,
    log: targetY.log,
    stripHeight,
    hasRows: input.rowLabelWidth !== undefined,
    hasExplicitLeftPadding: input.padding?.left != null,
    valueTicks: input.valueTicks,
    tokens: input.tokens,
    formatValue: input.formatValue,
    measure: input.measure,
  });
  return { frame, targetX, targetY, padding };
}

export interface TimeChartScene<C extends Canvas2D = Canvas2D> {
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly scaleX: number;
  readonly scaleY: number;
  readonly layers: readonly ChartLayer<C>[];
  readonly gaps?: readonly LostSpan[];
  readonly strip?: ChartStrip;
  readonly playhead: number | null;
  readonly playback: PlaybackMode;
  readonly afterPlayhead?: boolean;
  readonly until?: number | null;
  readonly markSummaries?: readonly MarkSummary[];
  readonly markSummariesMatched?: boolean;
  readonly badges?: readonly EventBadge[];
  readonly rowLabels?: readonly string[];
  readonly formatValue?: ChartAxisFormatterId | AxisValueFormatter;
  readonly overlay?: (ctx: C, frame: PlotFrame, tokens: ChartTokens) => void;
  readonly endLabels?: readonly LineEnd[];
  readonly endLabelGap?: number;
}

export function drawTimeChartScene<C extends Canvas2D>(
  ctx: C,
  frame: PlotFrame,
  tokens: ChartTokens,
  scene: TimeChartScene<C>,
  measure?: MeasureText,
): void {
  const measureText = measure ?? canvasMeasureText(ctx);
  const formatter = resolveAxisFormatter(scene.formatValue);
  const until = scene.until !== undefined
    ? scene.until
    : shownUntil(scene.playhead, scene.playback, scene.afterPlayhead === true);
  ctx.setTransform(scene.scaleX, 0, 0, scene.scaleY, 0, 0);
  ctx.clearRect(0, 0, scene.layoutWidth, scene.layoutHeight);
  drawAxes(ctx, frame, tokens, formatter, scene.rowLabels, measureText);
  const gaps = scene.gaps ?? [];
  for (const layer of scene.layers) {
    ctx.save();
    clipToPlot(ctx, frame);
    clipGaps(ctx, frame, layer.gaps ?? gaps);
    layer.draw(ctx, frame, tokens, { playhead: until });
    ctx.restore();
  }
  drawGaps(ctx, frame, tokens, gaps);
  if (scene.strip && frame.strip) {
    ctx.save();
    clipGaps(ctx, frame, gaps);
    scene.strip.draw(ctx, frame, tokens, { playhead: until });
    ctx.restore();
  }
  if (scene.playhead != null && scene.playback === 'cursor') drawPlayhead(ctx, frame, tokens, scene.playhead);
  if (scene.markSummaries) {
    for (const summary of scene.markSummaries) {
      if (scene.markSummariesMatched || markSummaryMatches(summary, frame, until)) drawMarkSummary(ctx, frame, tokens, summary, until);
    }
  }
  if (scene.badges) drawBadges(ctx, frame, tokens, scene.badges, until);
  scene.overlay?.(ctx, frame, tokens);
  if (scene.endLabels && scene.endLabels.length > 0) {
    placeEndLabels(ctx, frame, tokens, scene.endLabels, scene.endLabelGap ?? 2, measureText);
  }
}

export function chartTokensEqual(a: ChartTokens | null | undefined, b: ChartTokens | null | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.surface === b.surface &&
    a.raised === b.raised &&
    a.text === b.text &&
    a.muted === b.muted &&
    a.border === b.border &&
    a.grid === b.grid &&
    a.axis === b.axis &&
    a.accent === b.accent &&
    a.positive === b.positive &&
    a.warning === b.warning &&
    a.error === b.error &&
    a.blueprint === b.blueprint &&
    a.blueprintBorder === b.blueprintBorder &&
    a.blueprintText === b.blueprintText &&
    a.font === b.font &&
    a.fontSize.small === b.fontSize.small &&
    a.fontSize.body === b.fontSize.body &&
    a.fontSize.title === b.fontSize.title &&
    a.lineWidth.grid === b.lineWidth.grid &&
    a.lineWidth.series === b.lineWidth.series &&
    a.lineWidth.emphasis === b.lineWidth.emphasis &&
    a.hatchSpacing === b.hatchSpacing &&
    a.gapOpacity === b.gapOpacity &&
    a.motion === b.motion &&
    a.reducedMotion === b.reducedMotion &&
    a.countryColours === b.countryColours &&
    a.categories.length === b.categories.length &&
    a.categories.every((color, index) => color === b.categories[index])
  );
}
