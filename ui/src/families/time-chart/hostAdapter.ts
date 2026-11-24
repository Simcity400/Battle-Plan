import type { TiledLine } from '../../dashboard/tiles/view';
import { displayedHour } from '../boundedView.ts';
import type { FamilyGestureContext, FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { SerializedTiledLine } from '../sceneTiles.ts';
import type { SourcePlan } from '../contract.ts';
import type { TimeChartBinding } from './declaration.ts';
import { parseChartInspect } from './hitTest.ts';
import type { TimeChartModel } from './model.ts';

const MIN_ZOOM_SPAN_HOURS = 24;
const serializedLines = new WeakMap<TiledLine, SerializedTiledLine>();

function serializeTiledLine(source: string, line: TiledLine): SerializedTiledLine {
  const cached = serializedLines.get(line);
  if (cached && cached.source === source) return cached;
  const serialized: SerializedTiledLine = {
    source,
    tag: line.tag,
    head: line.head,
    decimals: line.decimals,
    binHours: line.summary.tiles?.binHours ?? 1,
    from: line.summary.from,
    to: line.summary.to,
    width: line.summary.width,
    buckets: line.summary.buckets,
    ...(line.summary.runs ? { runs: line.summary.runs } : {}),
    missing: line.missing,
    parts: line.parts,
  };
  serializedLines.set(line, serialized);
  return serialized;
}

type HeadSources = NonNullable<TimeChartBinding['headSourceBySeries']>;
const headSources = new WeakMap<SourcePlan, HeadSources | null>();

function headSourcesOf(plan: SourcePlan): HeadSources | null {
  let held = headSources.get(plan);
  if (held !== undefined) return held;
  const map: Record<string, HeadSources[string]> = {};
  for (const entry of plan.entries) {
    if (entry.read.shape === 'pixel-binned' && entry.read.headQuery) {
      map[entry.name] = {
        answerName: entry.name,
        tags: entry.read.tags,
        ...(entry.read.keys ? { keys: entry.read.keys } : {}),
        ...(entry.read.partBits !== undefined ? { partBits: entry.read.partBits } : {}),
      };
    }
  }
  held = Object.keys(map).length ? map : null;
  headSources.set(plan, held);
  return held;
}

const zooms = new WeakMap<readonly [number, number], { readonly x: readonly [number, number] }>();

function zoomOf(window: readonly [number, number]): { readonly x: readonly [number, number] } {
  let zoom = zooms.get(window);
  if (!zoom) zooms.set(window, zoom = { x: window });
  return zoom;
}

export function parseTimeChartHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  const parsed = parseChartInspect(hitKey);
  if (!parsed) return null;
  const hour = 'hour' in parsed ? parsed.hour : undefined;
  const subject = 'seriesId' in parsed ? parsed.seriesId : hitKey;
  return {
    key: hitKey,
    subject,
    ...(hour !== undefined ? { hour } : {}),
    kind: parsed.kind,
    toggle: false,
    inspectToken: hitKey,
  };
}

interface PlotRect {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

function plotRectOf(fields: Readonly<Record<string, unknown>>): PlotRect | null {
  const meta = fields.meta as { readonly plotRect?: PlotRect } | undefined;
  return meta?.plotRect && meta.plotRect.width > 0 ? meta.plotRect : null;
}

interface InstalledAxis {
  readonly shown: readonly [number, number];
  readonly full: readonly [number, number];
}

function installedAxisOf(fields: Readonly<Record<string, unknown>>): InstalledAxis | null {
  const meta = fields.meta as { readonly transform?: { readonly xFrom: number; readonly xTo: number; readonly axisFrom?: number; readonly axisTo?: number } } | undefined;
  const t = meta?.transform;
  if (!t || !Number.isFinite(t.xFrom) || !Number.isFinite(t.xTo) || t.axisFrom === undefined || t.axisTo === undefined) return null;
  if (!(t.axisTo > t.axisFrom) || !(t.xTo > t.xFrom)) return null;
  return { shown: [t.xFrom, t.xTo], full: [t.axisFrom, t.axisTo] };
}

function insidePlot(point: readonly [number, number], plot: PlotRect): boolean {
  return point[0] >= plot.left && point[0] <= plot.left + plot.width && point[1] >= plot.top && point[1] <= plot.top + plot.height;
}

function clampWindow(
  from: number,
  to: number,
  axis: readonly [number, number],
): readonly [number, number] | null {
  const fullSpan = Math.max(1, axis[1] - axis[0]);
  const span = Math.max(MIN_ZOOM_SPAN_HOURS, Math.min(fullSpan, to - from));
  if (span >= fullSpan - 0.5) return null;
  let lo = Math.max(axis[0], from);
  let hi = lo + span;
  if (hi > axis[1]) {
    hi = axis[1];
    lo = Math.max(axis[0], hi - span);
  }
  return [lo, hi];
}

interface DragState {
  readonly window: readonly [number, number];
  readonly axis: readonly [number, number];
  readonly plotWidth: number;
}

export function createTimeChartGestures() {
  const drags = new WeakMap<FamilyGestureContext['setZoomWindow'], DragState>();

  return {
    onWheel(event: WheelEvent, point: readonly [number, number], ctx: FamilyGestureContext) {
      const plot = plotRectOf(ctx.fields);
      const axis = installedAxisOf(ctx.fields);
      if (!plot || !axis || !insidePlot(point, plot)) return false;
      const fullSpan = axis.full[1] - axis.full[0];
      if (fullSpan <= MIN_ZOOM_SPAN_HOURS) return false;
      const [curFrom, curTo] = axis.shown;
      const frac = Math.max(0, Math.min(1, (point[0] - plot.left) / plot.width));
      const anchor = curFrom + frac * (curTo - curFrom);
      const factor = event.deltaY > 0 ? 1.25 : 0.8;
      const nextSpan = Math.max(MIN_ZOOM_SPAN_HOURS, Math.min(fullSpan, (curTo - curFrom) * factor));
      ctx.setZoomWindow(clampWindow(anchor - frac * nextSpan, anchor + (1 - frac) * nextSpan, axis.full));
      return true;
    },
    onDragStart(point: readonly [number, number], ctx: FamilyGestureContext) {
      const plot = plotRectOf(ctx.fields);
      const axis = installedAxisOf(ctx.fields);
      if (plot && axis && ctx.viewport.zoomWindow && insidePlot(point, plot)) {
        drags.set(ctx.setZoomWindow, { window: axis.shown, axis: axis.full, plotWidth: plot.width });
      } else {
        drags.delete(ctx.setZoomWindow);
      }
    },
    onDragMove(_point: readonly [number, number], delta: readonly [number, number], ctx: FamilyGestureContext) {
      const drag = drags.get(ctx.setZoomWindow);
      if (!drag || drag.plotWidth <= 0) return;
      const span = drag.window[1] - drag.window[0];
      const shiftHours = -(delta[0] / drag.plotWidth) * span;
      ctx.setZoomWindow(clampWindow(drag.window[0] + shiftHours, drag.window[1] + shiftHours, drag.axis));
    },
    onDragEnd(ctx: FamilyGestureContext) {
      drags.delete(ctx.setZoomWindow);
    },
    onDoubleClick(point: readonly [number, number], ctx: FamilyGestureContext) {
      const plot = plotRectOf(ctx.fields);
      if (!ctx.viewport.zoomWindow || !plot || !insidePlot(point, plot)) return false;
      ctx.setZoomWindow(null);
      return true;
    },
  };
}

export const timeChartHostAdapter: FamilyHostAdapter<TimeChartModel, unknown, TimeChartBinding> = {
  family: 'time-chart',
  pointerMode: 'continuous',
  navigation: 'points',
  parseHit: parseTimeChartHit,
  resolveHitAction(hit, installed, fields) {
    const target = installed.actionKeys.get(hit.key);
    const metaObj = fields.meta as { readonly subject?: string } | undefined;
    if (hit.kind === 'month') {
      return {
        installedId: installed.id,
        owner: installed.owner,
        subject: hit.subject ?? target?.subject ?? hit.key,
        hour: displayedHour(installed.hour),
        meta: {
          frameHour: installed.hour,
          ...(hit.hour !== undefined ? { monthFirstHour: hit.hour } : {}),
          ...(metaObj?.subject ? { country: metaObj.subject } : {}),
        },
      };
    }
    const label = hit.label ?? target?.label;
    const meta = hit.meta ?? target?.meta;
    return {
      installedId: installed.id,
      owner: installed.owner,
      subject: hit.subject ?? target?.subject ?? hit.key,
      hour: displayedHour(hit.hour ?? target?.hour ?? installed.hour),
      ...(hit.toggle || target?.toggle ? { toggle: true } : {}),
      ...(label !== undefined ? { label } : {}),
      ...(meta !== undefined ? { meta } : {}),
    };
  },
  inspectToken: key => key,
  gestures: createTimeChartGestures(),
  deriveAuditRows: model => model.auditRows ?? [],
  buildBinding: ctx => {
    if (!ctx.extent) return null;
    const tiledBySource: Record<string, Record<string, SerializedTiledLine>> = {};
    if (ctx.tiledSeries) {
      for (const [sourceName, lines] of Object.entries(ctx.tiledSeries)) {
        const byTag: Record<string, SerializedTiledLine> = {};
        for (const [tag, line] of lines) byTag[tag] = serializeTiledLine(sourceName, line);
        tiledBySource[sourceName] = byTag;
      }
    }
    const headSourceBySeries = ctx.plan ? headSourcesOf(ctx.plan) : null;
    return {
      extent: ctx.extent,
      ...(ctx.viewport.zoomWindow ? { zoom: zoomOf(ctx.viewport.zoomWindow) } : {}),
      tiledBySource,
      ...(headSourceBySeries ? { headSourceBySeries } : {}),
      ...(ctx.hovers && Object.keys(ctx.hovers).length ? { hovers: ctx.hovers } : {}),
    };
  },
};
