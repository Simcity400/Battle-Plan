import { useId, useLayoutEffect, useMemo, useRef } from 'react';
import type { Run } from '../../charts/series';
import { drawingSurface, sizeCanvas, watchCanvasSize } from '../../charts/surface';
import { canvasMeasureText } from '../../charts/textWidth.ts';
import { hideTooltip, showTooltip } from '../../charts/tooltip';
import { useFamilyTheme } from '../../families/host/theme.ts';
import { resolveTooltipEntryContent } from '../../families/host/tooltips.ts';
import { deriveTimeChartTooltip, hitTestTimeChart, parseChartInspect } from '../../families/time-chart/hitTest.ts';
import { hoverTimeChartLayout, layoutTimeChart, type TimeChartLayout } from '../../families/time-chart/layout.ts';
import type { TimeChartModel } from '../../families/time-chart/model.ts';
import { drawTimeChart } from '../../families/time-chart/painter.ts';
import { courseMissing, coursePoints, sharedSpans } from './landBattleCourse.ts';

export interface CourseSide {
  readonly id: string;
  readonly label: string;
  readonly color: string;
  readonly runs: readonly Run[];
}

const integer = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });

function courseChartModel(title: string, sides: readonly CourseSide[], extent: readonly [number, number]): TimeChartModel {
  const [from, until] = extent;
  const missing = sides.map(side => courseMissing(side.runs, from, until));
  const reason = `${title} not recorded`;
  return {
    kind: 'land-battle-course',
    measureTitle: title,
    mode: 'lines',
    hour: until,
    extent,
    yAxis: { format: 'number', min: 0, decimals: 0 },
    series: sides.map((side, index) => {
      const last = side.runs.at(-1);
      return {
        id: side.id, key: side.id, name: side.label, color: side.color, role: 'primary', step: true,
        points: coursePoints(side.runs, until),
        missingSpans: missing[index]!.map(span => ({ ...span, reason })),
        endLabel: last ? { name: side.label, value: integer.format(last.value) } : null,
      };
    }),
    gaps: missing.reduce((shared, spans) => sharedSpans(shared, spans)).map(span => ({ ...span, reason })),
  };
}

function spoken(model: TimeChartModel): string {
  const ends = model.series.map(series => `${series.name} ${series.endLabel?.value ?? 'not recorded'}`);
  return `${model.measureTitle} over the battle: ${ends.join(', ')}.`;
}

export default function BattleCourseChart({ title, sides, extent }: {
  readonly title: string;
  readonly sides: readonly CourseSide[];
  readonly extent: readonly [number, number];
}) {
  const { theme } = useFamilyTheme();
  const model = useMemo(() => courseChartModel(title, sides, extent), [title, sides, extent]);
  const canvas = useRef<HTMLCanvasElement>(null);
  const tooltipId = useId();
  const hover = useRef<{ layout: TimeChartLayout | null; inspect: string | null; paint: () => void }>({ layout: null, inspect: null, paint: () => {} });

  useLayoutEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const surface = drawingSurface(element);
    if (!surface.context) return;
    const context = surface.context;
    const measure = canvasMeasureText(context);
    const font = theme?.font ?? "'Segoe UI', system-ui, sans-serif";
    const paint = () => {
      const { layoutWidth, layoutHeight, backing } = size.read();
      sizeCanvas(surface.canvas, backing.width, backing.height);
      const base = layoutTimeChart(model, layoutWidth, layoutHeight, font, measure);
      hover.current.layout = base;
      const parsed = parseChartInspect(hover.current.inspect);
      const shown = parsed?.kind === 'label' || parsed?.kind === 'line' ? hoverTimeChartLayout(base, parsed.seriesId) : base;
      drawTimeChart(context, model, shown, theme, hover.current.inspect, backing.scaleX);
    };
    hover.current.paint = paint;
    const size = watchCanvasSize(element, paint);
    paint();
    return () => { size.disconnect(); hideTooltip(tooltipId); };
  }, [model, theme, tooltipId]);

  const leave = () => {
    hideTooltip(tooltipId);
    if (hover.current.inspect === null) return;
    hover.current.inspect = null;
    hover.current.paint();
  };

  return <figure className="lb-course">
    <figcaption>{title}</figcaption>
    <canvas ref={canvas} role="img" aria-label={spoken(model)} onPointerLeave={leave} onPointerMove={event => {
      const layout = hover.current.layout;
      if (!layout) return;
      const rect = event.currentTarget.getBoundingClientRect();
      const inspect = hitTestTimeChart(model, layout, [event.clientX - rect.left, event.clientY - rect.top]);
      if (inspect !== hover.current.inspect) {
        hover.current.inspect = inspect;
        hover.current.paint();
      }
      const entry = inspect ? deriveTimeChartTooltip(model, layout, inspect) : null;
      if (entry) showTooltip(tooltipId, resolveTooltipEntryContent(entry), event.clientX, event.clientY);
      else hideTooltip(tooltipId);
    }} />
  </figure>;
}
