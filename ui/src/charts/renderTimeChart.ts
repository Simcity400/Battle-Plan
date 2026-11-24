import { BADGE_STRIP, drawBadges, type EventBadge } from './badges.ts';
import { drawGapSummary, drawLineSummary, summaryExtent, type ChartSummary, type SummaryLineOptions } from './chartSummary.ts';
import {
  VALUE_LABEL_GAP,
  clipToPlot,
  drawAxes,
  drawPlayhead,
  drawValueRule,
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
import { formatAmount, formatAxisPercent, formatAxisValue, rescale } from './valueAxis.ts';
import { playbackDomain, resolveX, resolveY, shownUntil, type AxisSettings, type PlaybackMode, type Range, type ViewState } from './view.ts';

export const TIME_CHART_DEFAULT_PADDING: Padding = { left: 44, right: CHART_LAYOUT.right, top: 12, bottom: 24 };
export const TIME_CHART_LABEL_MARGIN = 4;
export const TIME_CHART_MIN_SPAN = 14 * 24;

export type ChartAxisFormatterId =
  | 'axisValue'
  | 'powerAxis'
  | 'percent'
  | 'stabilityPercent'
  | 'percentRaw'
  | 'apm'
  | 'fuel'
  | 'experienceBanked'
  | 'experienceRate';
export type ChartFitId = 'none' | 'stabilityZoom';
export type AxisValueFormatter = (value: number, ticks: readonly number[]) => string;
export type ValueFit = (shown: Range | null) => Range | null;

const FUEL_QUANTITY_SCALE = 32768;

export const powerAxisFormat: AxisValueFormatter = value =>
  Math.abs(value) >= 1000 ? `${Math.round(value / 100) / 10}k` : String(Math.round(value));

export const stabilityPercentAxisFormat: AxisValueFormatter = value => {
  const percent = value * 100;
  return `${Number.isInteger(Math.round(percent * 10) / 10) ? Math.round(percent) : percent.toFixed(1)}%`;
};

export const percentRawAxisFormat: AxisValueFormatter = value => `${value}%`;

export const apmAxisFormat: AxisValueFormatter = value =>
  `${value.toLocaleString(undefined, { maximumFractionDigits: 1 })} eAPM`;

export const fuelAxisFormat: AxisValueFormatter = value =>
  formatAmount(rescale(BigInt(Math.round(value * FUEL_QUANTITY_SCALE)), BigInt(FUEL_QUANTITY_SCALE), 0), 0, 0);

export const experienceBankedAxisFormat: AxisValueFormatter = value =>
  value.toLocaleString('en-US', { maximumFractionDigits: 2 });

export const experienceRateAxisFormat: AxisValueFormatter = value => {
  const text = value.toLocaleString('en-US', { maximumFractionDigits: 2 });
  return value > 0 ? `+${text}` : text;
};

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
  stabilityPercent: stabilityPercentAxisFormat,
  percentRaw: percentRawAxisFormat,
  apm: apmAxisFormat,
  fuel: fuelAxisFormat,
  experienceBanked: experienceBankedAxisFormat,
  experienceRate: experienceRateAxisFormat,
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

export type DeclarativeLayer =
  | {
      readonly kind: 'line-summary';
      readonly summary: ChartSummary | undefined;
      readonly color: string;
      readonly held?: boolean;
      readonly options?: SummaryLineOptions;
      readonly yScale?: number;
      readonly gaps?: readonly LostSpan[];
    }
  | {
      readonly kind: 'gap-summary';
      readonly first: ChartSummary | undefined;
      readonly second: ChartSummary | undefined;
      readonly aboveColor: string;
      readonly belowColor: string;
      readonly alpha?: number;
      readonly gaps?: readonly LostSpan[];
    }
  | {
      readonly kind: 'value-rule';
      readonly value: number;
      readonly gaps?: readonly LostSpan[];
    };

export interface ChartEndLabels {
  readonly ends: readonly LineEnd[];
  readonly gap?: number;
}

export type EndLabelsResolver = (frame: PlotFrame, until: number | null) => ChartEndLabels | readonly LineEnd[] | null | undefined;

export function normalizeEndLabels(resolved: ChartEndLabels | readonly LineEnd[] | null | undefined): ChartEndLabels | undefined {
  if (!resolved) return undefined;
  if (Array.isArray(resolved)) return resolved.length > 0 ? { ends: resolved } : undefined;
  const spec = resolved as ChartEndLabels;
  return spec.ends.length > 0 ? spec : undefined;
}

export interface ChartLayer<C extends Canvas2D = Canvas2D> {
  draw(ctx: C, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void;
  extent?(x: Range): Range | null;
  gaps?: readonly LostSpan[];
  describe?(): DeclarativeLayer;
}

export function scalePlotFrameY(frame: PlotFrame, scale: number): PlotFrame {
  if (scale === 1) return frame;
  return {
    ...frame,
    y: [frame.y[0] * scale, frame.y[1] * scale],
    toY: value => frame.toY(value / scale),
    valueAt: py => frame.valueAt(py) * scale,
  };
}

export interface LineSummaryLayerSpec {
  readonly summary: () => ChartSummary | undefined;
  readonly color: string;
  readonly held?: boolean;
  readonly options?: SummaryLineOptions;
  readonly until?: number | null;
  readonly yScale?: number;
  readonly gaps?: readonly LostSpan[];
  readonly withExtent?: boolean;
  readonly extent?: (x: Range) => Range | null;
}

export function lineSummaryLayer<C extends Canvas2D = Canvas2D>(spec: LineSummaryLayerSpec): ChartLayer<C> {
  const held = spec.held === true;
  const scale = spec.yScale ?? 1;
  const layer: ChartLayer<C> = {
    gaps: spec.gaps,
    draw(ctx, frame, tokens, state) {
      const summary = spec.summary();
      if (!summary) return;
      drawLineSummary(ctx, scale === 1 ? frame : scalePlotFrameY(frame, scale), tokens, summary, spec.color, held, state.playhead, spec.options);
    },
    describe: () => ({
      kind: 'line-summary',
      summary: spec.summary(),
      color: spec.color,
      held,
      options: spec.options,
      yScale: scale === 1 ? undefined : scale,
      gaps: spec.gaps,
    }),
  };
  if (spec.extent) {
    layer.extent = spec.extent;
  } else if (spec.withExtent !== false) {
    layer.extent = x => {
      const summary = spec.summary();
      if (!summary) return null;
      const range = summaryExtent(summary, x, spec.until ?? null);
      return range && scale !== 1 ? [range[0] / scale, range[1] / scale] : range;
    };
  }
  return layer;
}

export interface GapSummaryLayerSpec {
  readonly first: () => ChartSummary | undefined;
  readonly second: () => ChartSummary | undefined;
  readonly aboveColor: string;
  readonly belowColor: string;
  readonly alpha?: number;
  readonly gaps?: readonly LostSpan[];
}

export function gapSummaryLayer<C extends Canvas2D = Canvas2D>(spec: GapSummaryLayerSpec): ChartLayer<C> {
  return {
    gaps: spec.gaps,
    draw(ctx, frame, tokens, state) {
      const first = spec.first();
      const second = spec.second();
      if (first && second) drawGapSummary(ctx, frame, tokens, first, second, spec.aboveColor, spec.belowColor, state.playhead, spec.alpha === undefined ? undefined : { alpha: spec.alpha });
    },
    describe: () => ({
      kind: 'gap-summary',
      first: spec.first(),
      second: spec.second(),
      aboveColor: spec.aboveColor,
      belowColor: spec.belowColor,
      alpha: spec.alpha,
      gaps: spec.gaps,
    }),
  };
}

export function valueRuleLayer<C extends Canvas2D = Canvas2D>(value: number, gaps?: readonly LostSpan[]): ChartLayer<C> {
  return {
    gaps,
    draw(ctx, frame, tokens) {
      drawValueRule(ctx, frame, tokens, value);
    },
    describe: () => ({ kind: 'value-rule', value, gaps }),
  };
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
