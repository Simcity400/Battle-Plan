import { canvasMeasureText, type MeasureText } from '../../charts/textWidth.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { LaneFamily, LaneFrame } from '../laneRuntime.ts';
import { formatDateOnly } from '../valueFormat.ts';
import type { TimeChartBinding } from './declaration.ts';
import { deriveTimeChartTooltip, hitTestTimeChart, parseChartInspect } from './hitTest.ts';
import { layoutTimeChart, type TimeChartLayout } from './layout.ts';
import type { TimeChartModel } from './model.ts';
import { collectVisibleChartArt, drawTimeChart } from './painter.ts';

interface ChartLayout {
  readonly model: TimeChartModel;
  readonly base: TimeChartLayout | null;
  readonly width: number;
  readonly height: number;
  readonly font: string;
  readonly measure: MeasureText;
}

type ChartFrame = LaneFrame<TimeChartBinding, TimeChartModel, ChartLayout>;

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
  const stops: { readonly hour: number; readonly key: string; readonly name: string }[] = [];
  for (const tick of layout.axisTicks) {
    if (tick.hour <= model.hour) stops.push({ hour: tick.hour, key: `plot:${tick.hour}`, name: formatDateOnly(tick.hour) });
  }
  for (const placed of layout.markers) {
    stops.push({ hour: placed.marker.hour, key: `marker:${placed.index}@${placed.marker.hour}`, name: placed.marker.title });
  }
  stops.push({ hour: model.hour, key: `plot:${model.hour}`, name: formatDateOnly(model.hour) });
  stops.sort((a, b) => a.hour - b.hour);
  return stops.map((stop, index) => [stop.key, stop.name, index, 0, ''] as const);
}

const hovered = new WeakMap<ChartLayout, { readonly inspect: string; readonly layout: TimeChartLayout }>();

function shownLayout(layout: ChartLayout, inspect: string | null): TimeChartLayout | null {
  if (!layout.base) return null;
  const parsed = parseChartInspect(inspect);
  const hoverHour = parsed?.kind === 'plot' || parsed?.kind === 'line' ? parsed.hour : null;
  const seriesId = parsed?.kind === 'label' || parsed?.kind === 'line' ? parsed.seriesId : null;
  if (!inspect || (hoverHour === null && seriesId === null)) return layout.base;
  const held = hovered.get(layout);
  if (held && held.inspect === inspect) return held.layout;
  const shown = layoutTimeChart(layout.model, layout.width, layout.height, layout.font, layout.measure, hoverHour, seriesId);
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
    const base = model.empty ? null : layoutTimeChart(model, surface.width, surface.height, font, measureText, null, null);
    return { model, base, width: surface.width, height: surface.height, font, measure: measureText };
  },
  paint(ctx, frame) {
    const shown = shownOf(frame);
    if (!shown) {
      ctx.clearRect(0, 0, frame.surface.width, frame.surface.height);
      return false;
    }
    drawTimeChart(ctx, frame.layout.model, shown, frame.theme, frame.inspect, frame.surface.dpr > 0 ? frame.surface.dpr : 1);
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
        meta: {
          empty: model.empty?.message ?? null,
          emptyDetail: model.empty?.detail ?? null,
          emptyKind: model.empty?.kind ?? 'empty',
          notes: model.note ? [model.note] : [],
          warnings: model.warnings ?? [],
        },
      };
    }
    return {
      meta: {
        empty: null,
        emptyDetail: null,
        emptyKind: 'empty',
        ...(model.subject ? { subject: model.subject } : {}),
        notes: model.note ? [model.note] : [],
        warnings: model.warnings ?? [],
        transform: {
          xFrom: base.xDomain[0],
          xTo: base.xDomain[1],
          axisFrom: model.extent[0],
          axisTo: model.hour,
          yMin: base.yDomain[0],
          yMax: base.yDomain[1],
          log: model.yAxis.log === true,
        },
        plotRect: { left: base.left, top: base.top, width: base.plotWidth, height: base.plotHeight },
      },
      nav: chartNavStops(model, base),
    };
  },
  mirror: () => null,
  audit: frame => frame.layout.model.auditRows ?? [],
  visibleArt: frame => (frame.layout.base ? collectVisibleChartArt(frame.layout.model, frame.layout.base) : []),
};
