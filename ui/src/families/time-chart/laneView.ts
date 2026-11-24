import type { EventBadge } from '../../charts/badges.ts';
import {
  bucketsStream,
  buildSummaryShape,
  drawSummaryShape,
  type ChartBucket,
} from '../../charts/chartSummary.ts';
import type { LineEnd } from '../../charts/draw.ts';
import type { Padding, PlotFrame } from '../../charts/frame.ts';
import type { LostSpan } from '../../charts/gaps.ts';
import type { MarkSummary } from '../../charts/markSummary.ts';
import {
  TIME_CHART_DEFAULT_PADDING,
  chartTokensEqual,
  drawTimeChartScene,
  resolveTimeChartFrame,
  type ChartAxisFormatterId,
  type ChartFitId,
  type ChartLayer,
} from '../../charts/renderTimeChart.ts';
import { canvasMeasureText, type Canvas2D, type MeasureText } from '../../charts/textWidth.ts';
import { CHART_LAYOUT, type ChartTokens } from '../../charts/tokens.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { shownUntil, type AxisSettings, type PlaybackMode, type Range, type ViewState } from '../../charts/view.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { AHEAD_BINS, HELD_BINS_PER_PIXEL, cellBins, viewBinHours } from '../../dashboard/tiles/view.ts';
import type { LaneView, Painted, PaintInput, Value as LaneValue } from '../../draw/lane.ts';
import type { AuditRow } from '../../draw/notifications';
import type { HeadLine, SeriesHeadReply } from '../../generated/dashboard';
import { canonicalAuditData, type FamilyAuditField } from '../audit.ts';
import {
  ClosedBinRing,
  RetainedClosedWindowStore,
  TILE_BINS,
  closedBucketAt,
  closedEndValue,
  closedExtent,
  composeMarkSummary,
  headOpenBinAsBucket,
  promoteClosedBin,
  seriesKey,
  type DecodedTile,
  type RetainedClosedSeries,
  type SeriesKey,
} from './tiles.ts';

export interface TimeChartSeriesMeta {
  readonly key: SeriesKey;
  readonly country: string;
  readonly role: string;
  readonly label: string;
  readonly color: string;
  readonly dash?: readonly number[];
}

export interface TimeChartClosedTransfer {
  readonly key: SeriesKey;
  readonly binHours: number;
  readonly firstIndex: number;
  readonly lastIndex: number;
  readonly tiles: readonly DecodedTile[];
}

export interface TimeChartBinding {
  readonly family: 'time-chart';
  readonly kind: string;
  readonly tokens: ChartTokens;
  readonly extent: Range;
  readonly branchStart: number;
  readonly lastHour: number;
  readonly playback: PlaybackMode;
  readonly afterPlayhead: boolean;
  readonly windowHours: number;
  readonly paused: boolean;
  readonly playheadMode: 'follow' | 'pin' | 'range' | 'campaign';
  readonly staticPlayhead: number | null;
  readonly view: ViewState;
  readonly axes: AxisSettings;
  readonly padding?: Partial<Padding>;
  readonly formatValue?: ChartAxisFormatterId;
  readonly fitValues?: ChartFitId;
  readonly valueTicks?: readonly number[];
  readonly held?: boolean;
  readonly areaFill: boolean;
  readonly endLabels: boolean;
  readonly endLabelFormat: {
    readonly decimals: number | null;
    readonly digits: number;
    readonly unit: string;
  };
  readonly spendTicks: boolean;
  readonly series: readonly TimeChartSeriesMeta[];
  readonly badges?: readonly EventBadge[];
  readonly gaps?: readonly LostSpan[];
  readonly headSubId: number | null;
  readonly staticHead?: SeriesHeadReply | null;
  readonly tilesVersion: number;
  readonly closedTransfers?: readonly TimeChartClosedTransfer[];
  readonly audit?: boolean;
  readonly auditId?: number;
}

export interface TimeChartViewportField {
  readonly from: number;
  readonly to: number;
  readonly end: number;
  readonly width: number;
  readonly binHours: number;
  readonly firstTile: number;
  readonly lastTile: number;
  readonly cell: number;
}

export interface TimeChartLegendReading {
  readonly valueRaw: string | null;
  readonly value: number | null;
  readonly missing: string | null;
  readonly notApplicable: boolean | null;
}

export interface TimeChartFrameField {
  readonly x: Range;
  readonly y: Range;
  readonly log: boolean;
  readonly plotLeft: number;
  readonly plotTop: number;
  readonly plotWidth: number;
  readonly plotHeight: number;
  readonly until: number | null;
  readonly playhead: number | null;
  readonly markSummaries: readonly MarkSummary[];
}

interface HeldTimeChart {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  measure: MeasureText;
  measureFontKey: string;
  content: string;
  store: RetainedClosedWindowStore;
  rings: Map<SeriesKey, ClosedBinRing>;
  lastTilesVersion: number;
  transfersByKey: Map<SeriesKey, TimeChartClosedTransfer>;
  prevHead: SeriesHeadReply | null;
  lastTokens: ChartTokens | null;
  lastRenderSig: string;
  lastViewportSig: string;
  viewportField: { value: TimeChartViewportField; content: string } | null;
  lastLegendSig: string;
  legendField: { value: Readonly<Record<string, TimeChartLegendReading>>; content: string } | null;
  lastFrameSig: string;
  frameField: { value: TimeChartFrameField; content: string } | null;
  lastAuditSig: string;
  auditField: { value: FamilyAuditField; content: string } | null;
  heldViewBin: { binHours: number; width: number; cell: number } | null;
  markCache: Map<SeriesKey, { sig: string; summary: MarkSummary | null }>;
  lastMarkSummaries: readonly MarkSummary[];
}

export function isTimeChartBinding(value: unknown): value is TimeChartBinding {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'time-chart';
}

function isSeriesHeadReply(value: unknown): value is SeriesHeadReply {
  if (!value || typeof value !== 'object') return false;
  const obj = value as Partial<SeriesHeadReply>;
  return (
    typeof obj.branchId === 'string'
    && typeof obj.measure === 'string'
    && typeof obj.decimals === 'number'
    && typeof obj.hour === 'number'
    && typeof obj.binHours === 'number'
    && typeof obj.binFirstHour === 'number'
    && typeof obj.binLastHour === 'number'
    && Array.isArray(obj.lines)
  );
}

function headFromInput(input: PaintInput, binding: TimeChartBinding): { head: SeriesHeadReply | null; refused: string | null } {
  if (binding.headSubId === null) {
    return { head: binding.staticHead ?? null, refused: null };
  }
  const at = input.subs.findIndex(sub => sub.id === binding.headSubId);
  if (at < 0) return { head: binding.staticHead ?? null, refused: null };
  const val: LaneValue | undefined = input.pictures[at]?.parts[0]?.value;
  if (!val || val.kind !== 'panel') return { head: binding.staticHead ?? null, refused: null };
  if (val.refused !== null) return { head: null, refused: val.refused };
  return { head: isSeriesHeadReply(val.data) ? val.data : (binding.staticHead ?? null), refused: null };
}

function syncClosedRings(held: HeldTimeChart, head: SeriesHeadReply | null): void {
  const prev = held.prevHead;
  if (!head) {
    held.prevHead = null;
    return;
  }
  if (prev) {
    const resetRings =
      prev.branchId !== head.branchId
      || prev.binHours !== head.binHours
      || head.binFirstHour < prev.binFirstHour;
    if (resetRings) {
      for (const ring of held.rings.values()) ring.clear();
    } else if (head.binFirstHour > prev.binFirstHour) {
      for (const line of prev.lines) {
        const key = seriesKey(countryKey(line.country), 'primary');
        let ring = held.rings.get(key);
        if (!ring || ring.binHours !== prev.binHours) {
          ring = new ClosedBinRing(prev.binHours);
          held.rings.set(key, ring);
        }
        promoteClosedBin(ring, prev.binFirstHour, line.openBin, line.marks);
      }
    }
  }
  held.prevHead = head;
}

export function seriesLayerFor(
  closed: RetainedClosedSeries | null,
  head: SeriesHeadReply | null,
  headLine: HeadLine | null,
  color: string,
  heldMode: boolean,
  dash: readonly number[] | undefined,
  area: boolean,
  until: number | null,
): ChartLayer {
  const cut = until !== null && head ? head.binFirstHour : Infinity;
  return {
    extent(x: Range): Range | null {
      const end = Math.min(x[1], until ?? x[1]);
      if (end < x[0]) return null;
      let low = Infinity;
      let high = -Infinity;
      if (closed) {
        const range = closedExtent(closed, x[0], end, cut);
        if (range) {
          low = Math.min(low, range[0]);
          high = Math.max(high, range[1]);
        }
      }
      if (head && headLine?.openBin && (until === null || headLine.openBin.firstHour <= end) && headLine.openBin.lastHour >= x[0]) {
        if (until !== null || !closed || closedBucketAt(closed, head.binFirstHour) === null) {
          low = Math.min(low, headLine.openBin.minValue);
          high = Math.max(high, headLine.openBin.maxValue);
        }
      }
      return low <= high ? [low, high] : null;
    },
    draw(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void {
      const drawUntil = state.playhead ?? frame.x[1];
      const activeCut = state.playhead !== null && head ? head.binFirstHour : Infinity;
      const lineOpts = dash ? { dash } : { area };
      const drawOpen = !!(
        head
        && headLine?.openBin
        && headLine.openBin.firstHour <= drawUntil
        && (state.playhead !== null || !closed || closedBucketAt(closed, head.binFirstHour) === null)
      );
      if (closed) {
        const closedEnd = activeCut < Infinity ? Math.min(drawUntil, activeCut - 1) : drawUntil;
        const clipClosed = state.playhead !== null || activeCut < Infinity;
        if (!clipClosed || closedEnd >= frame.x[0]) {
          const closedHours: readonly [number, number] | undefined = clipClosed ? [frame.x[0], closedEnd] : undefined;
          drawSummaryShape(
            ctx,
            frame,
            tokens,
            closed.shape,
            color,
            closedHours ? { ...lineOpts, endDot: !drawOpen } : lineOpts,
            closedHours,
            { stream: closed.stream, held: heldMode, to: Infinity },
          );
        }
      }
      if (drawOpen && head && headLine?.openBin) {
        const openBucket = headOpenBinAsBucket(headLine.openBin);
        const buckets: ChartBucket[] = [];
        if (closed && !openBucket.firstBreak) {
          const prevBucket = closedBucketAt(closed, head.binFirstHour - head.binHours);
          if (prevBucket && prevBucket.lastHour + 1 === openBucket.firstHour) {
            buckets.push({
              firstHour: prevBucket.lastHour,
              lastHour: prevBucket.lastHour,
              lastValueHour: prevBucket.lastHour,
              minHour: prevBucket.lastHour,
              maxHour: prevBucket.lastHour,
              firstValue: prevBucket.lastValue,
              lastValue: prevBucket.lastValue,
              minValue: prevBucket.lastValue,
              maxValue: prevBucket.lastValue,
              firstBreak: false,
              hasBreak: false,
            });
          }
        }
        buckets.push(openBucket);
        const headStream = bucketsStream(buckets);
        const headShape = buildSummaryShape(headStream, heldMode, Infinity);
        drawSummaryShape(
          ctx,
          frame,
          tokens,
          headShape,
          color,
          { ...lineOpts, endDot: true },
          undefined,
          { stream: headStream, held: heldMode, to: Infinity },
        );
      }
    },
  };
}

function endValueFor(
  closed: RetainedClosedSeries | null,
  head: SeriesHeadReply | null,
  headLine: HeadLine | null,
  atHour: number,
  until: number | null,
): number | null {
  if (head && headLine?.openBin && headLine.openBin.firstHour <= atHour && (until === null || headLine.openBin.firstHour <= until)) {
    return headLine.openBin.lastValue;
  }
  if (closed) {
    const cut = until !== null && head ? head.binFirstHour : Infinity;
    return closedEndValue(closed, atHour, cut);
  }
  return null;
}

export function buildAuditRows(
  frame: PlotFrame,
  binHours: number,
  until: number | null,
  series: readonly TimeChartSeriesMeta[],
  retainedByKey: ReadonlyMap<SeriesKey, RetainedClosedSeries | null>,
  head: SeriesHeadReply | null,
  headByKey: ReadonlyMap<SeriesKey, HeadLine>,
  markSummaries: readonly MarkSummary[],
): AuditRow[] {
  const rows: AuditRow[] = [];
  rows.push({
    row: 'scale',
    key: 'scale',
    value: canonicalAuditData({
      x: [frame.x[0], frame.x[1]],
      y: [frame.y[0], frame.y[1]],
      log: frame.log,
      binHours,
      until,
    }),
  });
  const cut = until !== null && head ? head.binFirstHour : Infinity;
  for (let i = 0; i < series.length; i++) {
    const meta = series[i]!;
    const closed = retainedByKey.get(meta.key) ?? null;
    const headLine = headByKey.get(meta.key) ?? null;
    let visibleClosedCount = 0;
    let visibleMin: number | null = null;
    let visibleMax: number | null = null;
    let visibleLast: number | null = null;
    if (closed) {
      const startBin = Math.max(0, Math.floor((frame.x[0] - closed.firstBinHour) / closed.binHours));
      const endBin = Math.min(closed.binCount - 1, Math.floor((Math.min(frame.x[1], until ?? frame.x[1]) - closed.firstBinHour) / closed.binHours));
      for (let b = startBin; b <= endBin; b++) {
        if (closed.present[b] === 0) continue;
        if (closed.lastHours[b]! >= cut) break;
        if (closed.lastHours[b]! < frame.x[0]) continue;
        if (closed.firstHours[b]! > frame.x[1]) break;
        visibleClosedCount++;
        const lo = closed.minValues[b]!;
        const hi = closed.maxValues[b]!;
        visibleMin = visibleMin === null ? lo : Math.min(visibleMin, lo);
        visibleMax = visibleMax === null ? hi : Math.max(visibleMax, hi);
        visibleLast = closed.lastValues[b]!;
      }
    }
    const open = headLine?.openBin && headLine.openBin.lastHour >= frame.x[0] && headLine.openBin.firstHour <= Math.min(frame.x[1], until ?? frame.x[1])
      ? headLine.openBin
      : null;
    if (open) {
      visibleMin = visibleMin === null ? open.minValue : Math.min(visibleMin, open.minValue);
      visibleMax = visibleMax === null ? open.maxValue : Math.max(visibleMax, open.maxValue);
      visibleLast = open.lastValue;
    }
    let markDrops = 0n;
    for (const summary of markSummaries) {
      for (const c of summary.clusters) {
        if (c.tag === meta.country && c.lastHour >= frame.x[0] && c.firstHour <= Math.min(frame.x[1], until ?? frame.x[1])) {
          markDrops += BigInt(c.count);
        }
      }
    }
    rows.push({
      row: `series:${meta.key}`,
      key: `series:${meta.key}`,
      value: canonicalAuditData({
        country: meta.country,
        role: meta.role,
        headValueRaw: headLine?.value.valueRaw ?? null,
        headMissing: headLine?.value.missing ?? null,
        headNotApplicable: headLine?.value.notApplicable ?? null,
        openBin: open
          ? {
              firstHour: open.firstHour,
              lastHour: open.lastHour,
              firstValue: open.firstValue,
              lastValue: open.lastValue,
              minValue: open.minValue,
              maxValue: open.maxValue,
            }
          : null,
        visibleClosedCount,
        visibleMin,
        visibleMax,
        visibleLast,
        markDrops: markDrops.toString(),
      }),
    });
  }
  return rows;
}

export function timeChartLaneView(): LaneView {
  return {
    next: () => Infinity,
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const surface = input.binding.surfaces[0];
      if (!surface || !isTimeChartBinding(input.model)) {
        return { surfaces: [], fields: {}, motion: false };
      }
      const binding = input.model;
      const { head, refused } = headFromInput(input, binding);
      if (refused !== null) {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: refused };
      }

      const pixelWidth = Math.max(1, Math.round(surface.width * surface.dpr));
      const pixelHeight = Math.max(1, Math.round(surface.height * surface.dpr));
      let held = state.get('time-chart') as HeldTimeChart | undefined;
      if (!held) {
        const canvas = new OffscreenCanvas(pixelWidth, pixelHeight);
        const ctx = canvas.getContext('2d')!;
        const fontKey = `${binding.tokens.font}|${binding.tokens.fontSize.small}|${binding.tokens.fontSize.body}`;
        held = {
          canvas,
          ctx,
          measure: canvasMeasureText(ctx),
          measureFontKey: fontKey,
          content: content(),
          store: new RetainedClosedWindowStore(),
          rings: new Map(),
          lastTilesVersion: -1,
          transfersByKey: new Map(),
          prevHead: null,
          lastTokens: null,
          lastRenderSig: '',
          lastViewportSig: '',
          viewportField: null,
          lastLegendSig: '',
          legendField: null,
          lastFrameSig: '',
          frameField: null,
          lastAuditSig: '',
          auditField: null,
          heldViewBin: null,
          markCache: new Map(),
          lastMarkSummaries: [],
        };
        state.set('time-chart', held);
      }

      if (held.canvas.width !== pixelWidth || held.canvas.height !== pixelHeight) {
        held.canvas.width = pixelWidth;
        held.canvas.height = pixelHeight;
      }

      const fontKey = `${binding.tokens.font}|${binding.tokens.fontSize.small}|${binding.tokens.fontSize.body}`;
      if (held.measureFontKey !== fontKey) {
        held.measure = canvasMeasureText(held.ctx);
        held.measureFontKey = fontKey;
      }

      syncClosedRings(held, head);

      if (binding.closedTransfers && binding.tilesVersion !== held.lastTilesVersion) {
        held.lastTilesVersion = binding.tilesVersion;
        held.transfersByKey.clear();
        for (const transfer of binding.closedTransfers) {
          held.transfersByKey.set(transfer.key, transfer);
        }
      }

      const heldMode = binding.held === true;
      const retainedByKey = new Map<SeriesKey, RetainedClosedSeries | null>();
      const closedRevs: number[] = [];
      for (const meta of binding.series) {
        const transfer = held.transfersByKey.get(meta.key);
        const ring = held.rings.get(meta.key);
        const retained = transfer
          ? held.store.retain(meta.key, transfer.tiles, transfer.firstIndex, transfer.lastIndex, transfer.binHours, heldMode, ring)
          : null;
        retainedByKey.set(meta.key, retained);
        closedRevs.push(retained?.revision ?? 0);
      }

      const headByKey = new Map<SeriesKey, HeadLine>();
      if (head) {
        for (const line of head.lines) {
          headByKey.set(seriesKey(countryKey(line.country), 'primary'), line);
        }
      }

      const playhead =
        binding.playheadMode === 'follow'
          ? Math.max(binding.extent[0], Math.min(binding.extent[1], head?.hour ?? input.hour))
          : binding.staticPlayhead;
      const until = shownUntil(playhead, binding.playback, binding.afterPlayhead);

      const single = binding.series.length === 1;
      const layers: ChartLayer[] = binding.series.map(meta =>
        seriesLayerFor(
          retainedByKey.get(meta.key) ?? null,
          head,
          headByKey.get(meta.key) ?? null,
          meta.color,
          heldMode,
          meta.dash,
          single && binding.areaFill && meta.role === 'primary',
          until,
        ),
      );

      const resolved = resolveTimeChartFrame({
        layoutWidth: surface.width,
        layoutHeight: surface.height,
        tokens: binding.tokens,
        layers,
        extent: binding.extent,
        playhead,
        playback: binding.playback,
        windowHours: binding.windowHours,
        paused: binding.paused,
        view: binding.view,
        axes: binding.axes,
        branchStart: binding.branchStart,
        fitValues: binding.fitValues,
        formatValue: binding.formatValue,
        valueTicks: binding.valueTicks,
        hasBadges: !!binding.badges?.length,
        padding: binding.padding,
        measure: held.measure,
      });
      const frame = resolved.frame;

      const viewFrom = Math.floor(frame.x[0]);
      const viewTo = Math.ceil(Math.min(frame.x[1], until ?? frame.x[1]));
      const viewEnd = Math.ceil(frame.x[1]);
      const plotWidth = Math.max(1, Math.ceil(frame.plotWidth));
      const stableWidth = Math.max(1, Math.round(surface.width - TIME_CHART_DEFAULT_PADDING.left - (binding.padding?.right ?? CHART_LAYOUT.right)));
      const spanHours = Math.max(1, viewEnd - viewFrom + 1);
      const idealBinHours = viewBinHours(spanHours, stableWidth, held.heldViewBin ?? undefined);
      const binHours = head && binding.view.x === null && idealBinHours >= head.binHours && spanHours <= HELD_BINS_PER_PIXEL * stableWidth * head.binHours
        ? head.binHours
        : idealBinHours;
      const cell = cellBins(
        spanHours,
        stableWidth,
        binHours,
        held.heldViewBin && held.heldViewBin.binHours === binHours && held.heldViewBin.width === stableWidth
          ? held.heldViewBin.cell
          : undefined,
      );
      held.heldViewBin = { binHours, width: stableWidth, cell };
      const tileSpan = TILE_BINS * binHours;
      const firstTile = Math.floor(viewFrom / tileSpan);
      const ahead = Math.min(binding.lastHour, viewTo + AHEAD_BINS * binHours);
      const lastTile = Math.max(firstTile, Math.floor(Math.max(viewTo, ahead) / tileSpan));

      const nextMarkSummaries: MarkSummary[] = [];
      if (binding.spendTicks) {
        for (const meta of binding.series) {
          if (meta.role !== 'primary') continue;
          const closedSeries = retainedByKey.get(meta.key) ?? null;
          const hLine = headByKey.get(meta.key) ?? null;
          const color = single ? binding.tokens.muted : meta.color;
          const markSig = `${closedSeries?.revision ?? 0}|${head?.binFirstHour ?? ''}|${hLine?.marks?.before ?? ''}:${hLine?.marks?.count ?? 0}|${hLine?.marks?.count ? head?.hour ?? '' : ''}|${cell}|${color}`;
          const cached = held.markCache.get(meta.key);
          let summary: MarkSummary | null;
          if (cached && cached.sig === markSig) {
            summary = cached.summary;
          } else {
            summary = composeMarkSummary(
              closedSeries,
              head,
              hLine,
              viewFrom,
              viewEnd,
              plotWidth,
              cell,
              until,
              color,
              meta.country,
            );
            held.markCache.set(meta.key, { sig: markSig, summary });
          }
          if (summary && summary.clusters.length > 0) nextMarkSummaries.push(summary);
        }
      }
      const sameMarks = nextMarkSummaries.length === held.lastMarkSummaries.length
        && nextMarkSummaries.every((item, idx) => item === held.lastMarkSummaries[idx]);
      const markSummaries = sameMarks ? held.lastMarkSummaries : (held.lastMarkSummaries = nextMarkSummaries);

      const endLabels: LineEnd[] = [];
      const endDecimals = head?.decimals ?? binding.endLabelFormat.decimals;
      if (binding.endLabels && endDecimals !== null) {
        const at = Math.min(until ?? binding.extent[1], frame.x[1], binding.extent[1]);
        const scale = 10 ** endDecimals;
        for (const meta of binding.series) {
          if (meta.role !== 'primary') continue;
          const val = endValueFor(
            retainedByKey.get(meta.key) ?? null,
            head,
            headByKey.get(meta.key) ?? null,
            at,
            until,
          );
          if (val === null) continue;
          const raw = BigInt(Math.round(val * scale)).toString();
          const formatted = formatAmount(raw, endDecimals, binding.endLabelFormat.digits);
          const textPart = single
            ? { text: `${formatted} ${binding.endLabelFormat.unit}` }
            : { text: meta.label, suffix: formatted };
          endLabels.push({ id: meta.country, y: frame.toY(val), ...textPart, color: meta.color });
        }
      }

      const tokensMatch = chartTokensEqual(held.lastTokens, binding.tokens);
      const headSig = head
        ? `${head.branchId}:${head.hour}:${head.binFirstHour}:${head.lines.map(l => `${countryKey(l.country)}:${l.openBin?.lastHour ?? ''}:${l.openBin?.lastValue ?? ''}:${l.openBin?.minValue ?? ''}:${l.openBin?.maxValue ?? ''}:${l.marks?.count ?? ''}`).join(',')}`
        : 'none';
      const renderSig = JSON.stringify([
        pixelWidth,
        pixelHeight,
        surface.width,
        surface.height,
        frame.x,
        frame.y,
        frame.log,
        frame.plotLeft,
        frame.plotTop,
        frame.plotWidth,
        frame.plotHeight,
        playhead,
        until,
        binding.playback,
        binding.afterPlayhead,
        binding.areaFill,
        binding.endLabels,
        binding.endLabelFormat,
        binding.spendTicks,
        binding.formatValue ?? 'axisValue',
        closedRevs,
        headSig,
        binding.series.map(s => [s.key, s.label, s.color, s.dash]),
        binding.badges?.length ?? 0,
        binding.gaps?.length ?? 0,
      ]);

      if (!tokensMatch || renderSig !== held.lastRenderSig) {
        drawTimeChartScene(
          held.ctx,
          frame,
          binding.tokens,
          {
            layoutWidth: surface.width,
            layoutHeight: surface.height,
            scaleX: pixelWidth / surface.width,
            scaleY: pixelHeight / surface.height,
            layers,
            gaps: binding.gaps,
            playhead,
            playback: binding.playback,
            afterPlayhead: binding.afterPlayhead,
            until,
            markSummaries: markSummaries.length > 0 ? markSummaries : undefined,
            markSummariesMatched: true,
            badges: binding.badges,
            formatValue: binding.formatValue,
            endLabels: endLabels.length > 0 ? endLabels : undefined,
          },
          held.measure,
        );
        held.lastTokens = binding.tokens;
        held.lastRenderSig = renderSig;
        held.content = content();
      }

      const fields: Record<string, { value: unknown; content: string }> = {};

      const viewportSig = `${binHours}|${firstTile}|${lastTile}|${plotWidth}|${cell}`;
      if (viewportSig !== held.lastViewportSig || !held.viewportField) {
        held.lastViewportSig = viewportSig;
        held.viewportField = {
          value: { from: viewFrom, to: viewTo, end: viewEnd, width: plotWidth, binHours, firstTile, lastTile, cell },
          content: `vp:${content()}`,
        };
      }
      fields.viewport = held.viewportField;

      if (head) {
        const legendMap: Record<string, TimeChartLegendReading> = {};
        for (const line of head.lines) {
          legendMap[countryKey(line.country)] = {
            valueRaw: line.value.valueRaw,
            value: line.value.value,
            missing: line.value.missing ?? null,
            notApplicable: line.value.notApplicable ?? null,
          };
        }
        const legendSig = JSON.stringify(legendMap);
        if (legendSig !== held.lastLegendSig || !held.legendField) {
          held.lastLegendSig = legendSig;
          held.legendField = { value: legendMap, content: `legend:${content()}` };
        }
        fields.legend = held.legendField;
      }

      const frameSig = JSON.stringify([frame.x, frame.y, frame.log, frame.plotLeft, frame.plotTop, frame.plotWidth, frame.plotHeight, until, playhead, markSummaries.map(m => m.clusters.length)]);
      if (frameSig !== held.lastFrameSig || !held.frameField) {
        held.lastFrameSig = frameSig;
        held.frameField = {
          value: {
            x: frame.x,
            y: frame.y,
            log: frame.log,
            plotLeft: frame.plotLeft,
            plotTop: frame.plotTop,
            plotWidth: frame.plotWidth,
            plotHeight: frame.plotHeight,
            until,
            playhead,
            markSummaries,
          },
          content: `frame:${content()}`,
        };
      }
      fields.frame = held.frameField;

      if (binding.audit === true && binding.auditId !== undefined) {
        const auditRows = buildAuditRows(frame, head?.binHours ?? binHours, until, binding.series, retainedByKey, head, headByKey, markSummaries);
        const auditSig = `${binding.auditId}:${JSON.stringify(auditRows)}`;
        if (auditSig !== held.lastAuditSig || !held.auditField) {
          held.lastAuditSig = auditSig;
          held.auditField = {
            value: { id: binding.auditId, rows: auditRows },
            content: `audit:${content()}`,
          };
        }
        fields.audit = held.auditField;
      }

      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
      };
    },
    unmount(_widget, state) {
      state.delete('time-chart');
    },
  };
}
