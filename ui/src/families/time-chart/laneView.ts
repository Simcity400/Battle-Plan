import { canvasMeasureText, type MeasureText } from '../../charts/textWidth.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { LaneFamily, LaneFrame } from '../laneRuntime.ts';
import { formatDateOnly } from '../valueFormat.ts';
import type { TimeChartBinding } from './declaration.ts';
import { deriveTimeChartTooltip, hitTestTimeChart, parseChartInspect } from './hitTest.ts';
import { hoverTimeChartLayout, layoutTimeChart, type TimeChartLayout } from './layout.ts';
import type { TimeChartModel } from './model.ts';
import { collectVisibleChartArt, drawTimeChart, fitTimeChartLegend } from './painter.ts';
import type { LegendFit, TimeAxisTick } from '../../draw/kit.ts';
import { timeChartCacheState } from './cacheControl.ts';

interface ChartLayout {
  readonly model: TimeChartModel;
  readonly base: TimeChartLayout | null;
  readonly width: number;
  readonly height: number;
  readonly font: string;
  readonly measure: MeasureText;
  readonly legend: LegendFit | null;
}

type ChartFrame = LaneFrame<TimeChartBinding, TimeChartModel, ChartLayout>;

function collectChangeHours(model: TimeChartModel, layout: TimeChartLayout): number[] {
  const minHour = layout.xDomain[0];
  const maxHour = Math.min(layout.xDomain[1], model.hour);
  const hours = new Set<number>();
  if (model.mode === 'lines') {
    for (const s of model.series) {
      if (!s.points?.length) continue;
      let prev: number | null = null;
      let started = false;
      for (const pt of s.points) {
        if (pt.hour > maxHour) break;
        const val = pt.value !== null && Number.isFinite(pt.value) ? pt.value : null;
        if (pt.hour >= minHour && ((!started && val !== null) || (started && val !== prev))) {
          hours.add(pt.hour);
        }
        prev = val;
        started = true;
      }
    }
  } else if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    const { hours: stackHours, bands } = model.stack;
    for (const band of bands) {
      let prevActive = false;
      for (let i = 0; i < stackHours.length; i++) {
        const h = stackHours[i]!;
        if (h > maxHour) break;
        const v = band.values[i];
        const active = v !== null && v !== undefined && Number.isFinite(v) && v !== 0;
        if (h >= minHour && active !== prevActive) hours.add(h);
        prevActive = active;
      }
    }
  }
  return [...hours].sort((a, b) => a - b);
}

interface CachedNavPrefix {
  readonly rawHours: readonly number[];
  readonly maxTickHour: number;
  readonly prefix: readonly FamilyNavTuple[];
}

const NAV_PREFIX_CACHE_MAX = 16;
const navPrefixCache: CachedNavPrefix[] = [];

function sameTickHours(ticks: readonly TimeAxisTick[], hours: readonly number[]): boolean {
  if (ticks.length !== hours.length) return false;
  for (let i = 0; i < ticks.length; i++) if (ticks[i]!.hour !== hours[i]) return false;
  return true;
}

function hasPointSeries(model: TimeChartModel): boolean {
  for (let i = 0; i < model.series.length; i++) {
    if (model.series[i]!.points?.length) return true;
  }
  return false;
}

export function chartNavStops(model: TimeChartModel, layout: TimeChartLayout): readonly FamilyNavTuple[] {
  if (model.mode === 'columns') {
    return layout.columns.map((col, index) => {
      if (col.kind === 'fold') {
        const hour = col.foldMonths?.[0]?.firstHour;
        const key = hour !== undefined ? `fold:${col.index}@${hour}` : `fold:${col.index}`;
        return [key, col.foldSpanText ?? col.foldLabel ?? 'Folded months', index, 0, ''] as const;
      }
      const m = col.month!;
      const key = `month:${col.index}@${m.firstHour}`;
      const name = formatDateOnly(m.firstHour).replace(/^\d+\s+/, '');
      return [key, name, index, 0, ''] as const;
    });
  }
  if (timeChartCacheState.enabled && model.mode === 'lines' && layout.markers.length === 0 && !hasPointSeries(model)) {
    const ticks = layout.rawAxisTicks;
    for (let i = navPrefixCache.length - 1; i >= 0; i--) {
      const entry = navPrefixCache[i]!;
      if (entry.maxTickHour < model.hour && sameTickHours(ticks, entry.rawHours)) {
        const out = entry.prefix.slice();
        out.push([`plot:${model.hour}`, layout.dateTabText || formatDateOnly(model.hour), out.length, 0, ''] as const);
        return out;
      }
    }
    if (ticks.length === 0 || ticks[ticks.length - 1]!.hour < model.hour) {
      const seen = new Set<number>();
      const prefixHours: number[] = [];
      const rawHours: number[] = new Array(ticks.length);
      for (let i = 0; i < ticks.length; i++) {
        const h = ticks[i]!.hour;
        rawHours[i] = h;
        if (h < model.hour && !seen.has(h)) {
          seen.add(h);
          prefixHours.push(h);
        }
      }
      prefixHours.sort((a, b) => a - b);
      const prefix: FamilyNavTuple[] = prefixHours.map((h, index) => [`plot:${h}`, formatDateOnly(h), index, 0, ''] as const);
      const maxTickHour = prefixHours.length > 0 ? prefixHours[prefixHours.length - 1]! : -Infinity;
      if (ticks.length > 0) {
        if (navPrefixCache.length >= NAV_PREFIX_CACHE_MAX) navPrefixCache.shift();
        navPrefixCache.push({ rawHours, maxTickHour, prefix });
      }
      const out = prefix.slice();
      out.push([`plot:${model.hour}`, layout.dateTabText || formatDateOnly(model.hour), out.length, 0, ''] as const);
      return out;
    }
  }
  const stops: { readonly hour: number; readonly key: string; readonly name: string }[] = [];
  const plotHours = new Set<number>();
  const addPlot = (hour: number) => {
    if (plotHours.has(hour)) return;
    plotHours.add(hour);
    stops.push({ hour, key: `plot:${hour}`, name: formatDateOnly(hour) });
  };
  for (const tick of layout.rawAxisTicks) {
    if (tick.hour < model.hour) addPlot(tick.hour);
  }
  for (const hour of collectChangeHours(model, layout)) {
    if (hour < model.hour) addPlot(hour);
  }
  for (const placed of layout.markers) {
    stops.push({ hour: placed.marker.hour, key: `marker:${placed.index}@${placed.marker.hour}`, name: placed.marker.title });
  }
  addPlot(model.hour);
  stops.sort((a, b) => a.hour - b.hour);
  return stops.map((stop, index) => [stop.key, stop.name, index, 0, ''] as const);
}

export const LOG_REFUSED = 'Log scale needs every value shown above zero, so the value axis is linear.';

const hovered = new WeakMap<ChartLayout, { readonly inspect: string; readonly layout: TimeChartLayout }>();

function shownLayout(layout: ChartLayout, inspect: string | null): TimeChartLayout | null {
  if (!layout.base) return null;
  const parsed = parseChartInspect(inspect);
  const hoverHour = parsed?.kind === 'plot' || parsed?.kind === 'line' ? parsed.hour : null;
  const seriesId = parsed?.kind === 'label' || parsed?.kind === 'line' ? parsed.seriesId : null;
  if (!inspect || (hoverHour === null && seriesId === null)) return layout.base;
  const held = hovered.get(layout);
  if (held && held.inspect === inspect) return held.layout;
  const shown = hoverTimeChartLayout(layout.base, seriesId);
  hovered.set(layout, { inspect, layout: shown });
  return shown;
}

const shownOf = (frame: ChartFrame) => shownLayout(frame.layout, frame.inspect);

export const timeChartLane: LaneFamily<TimeChartBinding, TimeChartModel, ChartLayout> = {
  family: 'time-chart',
  boundary: (_model, hour) => hour + 1,
  redraw: () => Infinity,
  status: () => 'scene',
  view: ['zoom'],
  layout(projected, binding, surface, theme, measure) {
    const zoom = binding.zoom?.x;
    const model = zoom && projected.view?.x !== zoom ? { ...projected, view: { ...projected.view, x: zoom } } : projected;
    const font = theme.font ?? "'Segoe UI', system-ui, sans-serif";
    const measureText = canvasMeasureText(measure);
    const base = model.empty ? null : layoutTimeChart(model, surface.width, surface.height, font, measureText);
    const legend = base ? fitTimeChartLegend(model, base, theme, measureText) : null;
    return { model, base, width: surface.width, height: surface.height, font, measure: measureText, legend };
  },
  paint(ctx, frame) {
    const shown = shownOf(frame);
    if (!shown) {
      ctx.clearRect(0, 0, frame.surface.width, frame.surface.height);
      return false;
    }
    drawTimeChart(ctx, frame.layout.model, shown, frame.theme, frame.inspect, frame.surface.dpr > 0 ? frame.surface.dpr : 1, frame.layout.legend, frame.art);
    return false;
  },
  hit(frame, point) {
    const shown = shownOf(frame);
    return shown ? hitTestTimeChart(frame.layout.model, shown, point) : null;
  },
  tooltip(frame, key, focus) {
    const shown = focus ? frame.layout.base : shownOf(frame);
    return shown ? deriveTimeChartTooltip(frame.layout.model, shown, key) : null;
  },
  structure({ layout }) {
    const { model, base } = layout;
    if (!base || model.empty) {
      return {
        drawnKeys: model.drawnKeys ?? { main: [] },
        meta: {
          empty: model.empty?.message ?? null,
          emptyDetail: model.empty?.detail ?? null,
          emptyKind: model.empty?.kind ?? 'empty',
          summaryText: model.summaryText ?? null,
          notes: model.note ? [model.note] : [],
          warnings: model.warnings ?? [],
        },
      };
    }
    return {
      drawnKeys: model.drawnKeys ?? { main: (model.stack?.bands ?? model.series).map(line => line.key) },
      meta: {
        empty: null,
        emptyDetail: null,
        emptyKind: 'empty',
        ...(model.subject ? { subject: model.subject } : {}),
        summaryText: model.summaryText ?? null,
        notes: [...(model.note ? [model.note] : []), ...(base.logRefused ? [LOG_REFUSED] : [])],
        warnings: model.warnings ?? [],
        transform: {
          xFrom: base.xDomain[0],
          xTo: base.xDomain[1],
          axisFrom: model.extent[0],
          axisTo: model.hour,
          yMin: base.yDomain[0],
          yMax: base.yDomain[1],
          log: base.logShown,
        },
        plotRect: { left: base.left, top: base.top, width: base.plotWidth, height: base.plotHeight },
      },
      nav: chartNavStops(model, base),
    };
  },
  mirror: null,
  audit: frame => frame.layout.model.auditRows ?? [],
  visibleArt: frame => (frame.layout.base ? collectVisibleChartArt(frame.layout.model, frame.layout.base, frame.layout.legend) : []),
};
