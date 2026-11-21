import { useMemo, useRef } from 'react';
import { drawBands, drawPlayhead } from '../draw';
import { clipToSpans, runAt } from '../series';
import type { Run, Span } from '../series';
import { exactRuns, summaryStack, summaryFits } from '../chartSummary';
import type { ChartSummary } from '../chartSummary';
import { compactStackExtent, forEachCompactStackStretch, buildStack, stackExtent } from '../stack';
import { countryRaceSegments } from '../race/countryRace';
import { shownUntil } from '../view';
import TimeChart from '../TimeChart';
import type { TimeChartProps } from '../TimeChart';
import type { TooltipContent } from '../tooltip';
import { fitTooltip } from '../tooltip';

export interface BreakdownSeries { id: string; label: string; color: string; runs: readonly Run[];
  amount: (hour: number) => string; labelAt?: (hour: number) => string; missingAt?: (hour: number) => string }

export default function BreakdownChart({ series, within, selectedHour, tooltipHeight, summaries, overlayExtent, ...props }: Omit<TimeChartProps, 'layers' | 'tooltip' | 'revision'> & {
  series: readonly BreakdownSeries[]; within?: readonly Span[] | null; selectedHour?: number | null;
  overlayExtent?: (x: readonly [number, number]) => [number, number] | null;
  tooltipHeight?: number; summaries?: { parts: readonly ChartSummary[]; total?: ChartSummary };
}) {
  const parts = useMemo(() => series.map(part => {
    const runs = clipToSpans(part.runs, [{ firstHour: props.extent[0], lastHour: props.extent[1] }]);
    return { ...part, runs: within ? clipToSpans(runs, within) : runs };
  }), [series, within, props.extent]);
  const stack = useMemo(() => buildStack(parts.map(part => part.runs)), [parts]);
  const dense = useMemo(() => summaries && summaries.parts.length === parts.length
    && summaries.parts.some(part => exactRuns(part) === null)
    ? summaryStack(summaries.total ? summaries.parts.slice(0, -1) : summaries.parts, summaries.total) : null, [parts, summaries]);
  const until = shownUntil(props.playhead, props.playback, props.afterPlayhead === true);
  const live = props.liveHead;
  const reach = useRef(() => until);
  reach.current = () => live && props.playhead != null
    ? shownUntil(Math.max(props.extent[0], Math.min(props.extent[1], live.read())), props.playback, props.afterPlayhead === true) : until;
  const layers = useMemo(() => [{
    draw: (ctx: CanvasRenderingContext2D, frame: Parameters<typeof drawBands>[1], tokens: Parameters<typeof drawBands>[2], state: { playhead: number | null }) => {
      if (dense) {
        const view = summaries!.parts[0];
        if (!summaryFits(frame, view, state.playhead ?? null)) return;
        forEachCompactStackStretch(dense.model, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
          (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth), segment => {
            drawBands(ctx, frame, tokens, segment.hours, segment.bands, parts.map(part => part.color), segment);
          }, dense.sample);
        return;
      }
      for (const segment of countryRaceSegments(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
        (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth))) {
        drawBands(ctx, frame, tokens, segment.hours, segment.bands, parts.map(part => part.color));
      }
    },
    extent: (x: readonly [number, number]) => {
      const shown = reach.current();
      const end = Math.min(x[1], shown ?? x[1]);
      if (end < x[0]) return null;
      const own = dense ? compactStackExtent(dense.model, x[0], end) : stackExtent(stack, x[0], end);
      const extra = overlayExtent?.([x[0], end]) ?? null;
      return own && extra ? [Math.min(own[0], extra[0]), Math.max(own[1], extra[1])] as [number, number] : own ?? extra;
    },
  }], [stack, dense, parts, until, summaries, overlayExtent]);
  return <div className="breakdown-chart">
    <TimeChart {...props} layers={layers} keyHours={props.keyHours ?? dense?.model.hours ?? stack.hours} revision={parts}
    overlay={(ctx, frame, tokens) => {
      props.overlay?.(ctx, frame, tokens);
      const shown = reach.current();
      if (selectedHour != null && selectedHour >= props.extent[0] && selectedHour <= props.extent[1]
        && (shown === null || selectedHour <= shown)) drawPlayhead(ctx, frame, tokens, selectedHour);
    }}
    tooltip={hour => {
      if (hour < props.extent[0] || hour > props.extent[1]) return null;
      const content = { title: props.label, rows: parts.map(part => ({ label: part.labelAt?.(hour) ?? part.label, swatch: part.color,
        value: runAt(part.runs, hour) ? part.amount(hour) : part.missingAt?.(hour) ?? 'Not recorded' })) } satisfies TooltipContent;
      return tooltipHeight === undefined ? content : fitTooltip([content], tooltipHeight);
    }} /></div>;
}
