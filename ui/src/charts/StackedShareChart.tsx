import { useLayoutEffect, useMemo } from 'react';
import Legend from './Legend';
import { SHARE_TICKS, shareLayer, shareStack, summaryShareRuns } from './share';
import type { ChartSummary } from './chartSummary';
import { drawApproximateSpans, summaryStack } from './chartSummary';
import type { ShareBand, ShareRun } from './share';
import TimeChart from './TimeChart';
import type { TimeChartProps } from './TimeChart';
import type { ChartLayer } from './TimeChart';
import { formatAxisPercent } from './valueAxis';

export type { ShareBand } from './share';
type Props = Omit<TimeChartProps, 'layers' | 'axes' | 'formatValue' | 'valueTicks'> & {
  runs: readonly ShareRun[];
  /** Bottom to top; the legend names the top band first. */
  bands: readonly ShareBand[];
  summaries?: readonly ChartSummary[];
};

/** Theme-coloured stacked shares with held values, gaps and a quarter axis. */
export default function StackedShareChart({ runs, bands, summaries, ...chart }: Props) {
  const model = useMemo(() => shareStack(summaryShareRuns(runs, summaries), bands.length), [runs, bands.length, summaries]);
  const [first, last] = chart.extent;
  const layers = useMemo(() => [shareLayer(model, bands, [first, last])], [bands, first]);
  const approximate = useMemo(() => summaryStack(summaries ?? []).approximate, [summaries]);
  const drawn = useMemo<ChartLayer[]>(() => approximate.length ? [{ ...layers[0], draw: (ctx, frame, tokens, state) =>
    drawApproximateSpans(ctx, frame, approximate, state.playhead, () => layers[0].draw(ctx, frame, tokens, state)) }] : layers, [layers, approximate]);
  useLayoutEffect(() => { layers[0].update(model, [first, last]); }, [layers, model, first, last]);
  // The layer takes each model in place, so the chart repaints by the model too.
  const revision = useMemo(() => [chart.revision, model], [chart.revision, model]);
  return <div className="stacked-share-chart">
    <Legend label="Share bands" items={[...bands].reverse().map(band => ({ ...band }))} />
    {!!approximate.length && <p className="chart-line-key"><i className="is-dashed" aria-hidden="true" />Approximate</p>}
    <div className="stacked-share-plot">
      <TimeChart {...chart} revision={revision} layers={drawn} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: 100, log: false }}
        formatValue={formatAxisPercent} valueTicks={SHARE_TICKS} />
    </div>
  </div>;
}
