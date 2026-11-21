import { useLayoutEffect, useMemo, useRef } from 'react';
import Legend from './Legend';
import { SHARE_TICKS, liveShareLayer, shareLayer, shareStack, summaryShareRuns } from './share';
import type { ChartSummary } from './chartSummary';
import type { LiveShares, ShareBand, ShareRun } from './share';
import TimeChart from './TimeChart';
import type { ChartData, TimeChartProps } from './TimeChart';
import { formatAxisPercent } from './valueAxis';

export type { ShareBand } from './share';
type Props = Omit<TimeChartProps, 'layers' | 'axes' | 'formatValue' | 'valueTicks' | 'data'> & {
  runs: readonly ShareRun[];
  bands: readonly ShareBand[];
  summaries?: readonly ChartSummary[];
  live?: LiveShares;
};

export default function StackedShareChart({ runs, bands, summaries, live, ...chart }: Props) {
  const model = useMemo(() => shareStack(summaryShareRuns(runs, summaries), bands.length), [runs, bands.length, summaries]);
  const [first, last] = chart.extent;
  const held = useRef({ extent: [first, last] as readonly [number, number], summaries }); held.current = { extent: [first, last], summaries };
  const base = useMemo(() => shareLayer(model, bands, [first, last]), [bands, first]);
  const layers = useMemo(() => [live ? liveShareLayer(base, live, bands.length, () => held.current.extent, () => held.current.summaries) : base],
    [base, live, bands.length]);
  useLayoutEffect(() => { if (!live) base.update(model, [first, last]); }, [base, live, model, first, last]);
  const revision = useMemo(() => [chart.revision, model], [chart.revision, model]);
  const data = useMemo<ChartData | undefined>(() => live ? { revision: () => [live.revision()], subscribe: live.subscribe } : undefined, [live]);
  return <div className="stacked-share-chart">
    <Legend label="Share bands" items={[...bands].reverse().map(band => ({ ...band }))} />
    <div className="stacked-share-plot">
      <TimeChart {...chart} revision={revision} layers={layers} data={data} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: 100, log: false }}
        formatValue={formatAxisPercent} valueTicks={SHARE_TICKS} />
    </div>
  </div>;
}
