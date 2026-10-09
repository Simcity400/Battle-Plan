import { useMemo } from 'react';
import { drawBands, drawPlayhead } from '../draw';
import { clipToSpans, runAt } from '../series';
import type { Run, Span } from '../series';
import { buildStack, stackExtent } from '../stack';
import { countryRaceSegments } from '../race/countryRace';
import { shownUntil } from '../view';
import TimeChart from '../TimeChart';
import type { TimeChartProps } from '../TimeChart';
import type { TooltipContent } from '../tooltip';
import { fitTooltip } from '../tooltip';

export interface BreakdownSeries { id: string; label: string; color: string; runs: readonly Run[];
  amount: (hour: number) => string; labelAt?: (hour: number) => string; missingAt?: (hour: number) => string }

/** Parts share the same signed stack and exact hover amounts. Picking a day
 * changes the drill state, never the dashboard clock. */
export default function BreakdownChart({ series, within, selectedHour, tooltipHeight, ...props }: Omit<TimeChartProps, 'layers' | 'tooltip' | 'revision'> & {
  series: readonly BreakdownSeries[]; within?: readonly Span[] | null; selectedHour?: number | null;
  /** Fit long cohorts to the viewport, retaining all exact amounts in spoken text. */
  tooltipHeight?: number;
}) {
  const parts = useMemo(() => series.map(part => {
    const runs = clipToSpans(part.runs, [{ firstHour: props.extent[0], lastHour: props.extent[1] }]);
    return { ...part, runs: within ? clipToSpans(runs, within) : runs };
  }), [series, within, props.extent]);
  const stack = useMemo(() => buildStack(parts.map(part => part.runs)), [parts]);
  const until = shownUntil(props.playhead, props.playback, props.afterPlayhead === true);
  const layers = useMemo(() => [{
    draw: (ctx: CanvasRenderingContext2D, frame: Parameters<typeof drawBands>[1], tokens: Parameters<typeof drawBands>[2], state: { playhead: number | null }) => {
      for (const segment of countryRaceSegments(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
        (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth))) {
        drawBands(ctx, frame, tokens, segment.hours, segment.bands, parts.map(part => part.color));
      }
    },
    extent: (x: readonly [number, number]) => {
      const end = Math.min(x[1], until ?? x[1]);
      return end >= x[0] ? stackExtent(stack, x[0], end) : null;
    },
  }], [stack, parts, until]);
  return <div className="breakdown-chart"><TimeChart {...props} layers={layers} keyHours={props.keyHours ?? stack.hours} revision={parts}
    overlay={(ctx, frame, tokens) => {
      props.overlay?.(ctx, frame, tokens);
      if (selectedHour != null && selectedHour >= props.extent[0] && selectedHour <= props.extent[1]
        && (until === null || selectedHour <= until)) drawPlayhead(ctx, frame, tokens, selectedHour);
    }}
    tooltip={hour => {
      if (hour < props.extent[0] || hour > props.extent[1]) return null;
      const content = { title: props.label, rows: parts.map(part => ({ label: part.labelAt?.(hour) ?? part.label, swatch: part.color,
        value: runAt(part.runs, hour) ? part.amount(hour) : part.missingAt?.(hour) ?? 'Not recorded' })) } satisfies TooltipContent;
      return tooltipHeight === undefined ? content : fitTooltip([content], tooltipHeight);
    }} /></div>;
}
