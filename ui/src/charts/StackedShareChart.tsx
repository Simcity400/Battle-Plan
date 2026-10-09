import { useLayoutEffect, useMemo } from 'react';
import Legend from './Legend';
import { SHARE_TICKS, shareLayer, shareStack } from './share';
import type { ShareBand, ShareRun } from './share';
import TimeChart from './TimeChart';
import type { TimeChartProps } from './TimeChart';
import { formatAxisPercent } from './valueAxis';

export type { ShareBand } from './share';
type Props = Omit<TimeChartProps, 'layers' | 'axes' | 'formatValue' | 'valueTicks'> & {
  runs: readonly ShareRun[];
  /** Bottom to top; the legend names the top band first. */
  bands: readonly ShareBand[];
};

/** Theme-coloured stacked shares with held values, gaps and a quarter axis. */
export default function StackedShareChart({ runs, bands, ...chart }: Props) {
  const model = useMemo(() => shareStack(runs, bands.length), [runs, bands.length]);
  const [first, last] = chart.extent;
  const layers = useMemo(() => [shareLayer(model, bands, [first, last])], [bands, first]);
  useLayoutEffect(() => { layers[0].update(model, [first, last]); }, [layers, model, first, last]);
  return <div className="stacked-share-chart">
    <Legend label="Share bands" items={[...bands].reverse().map(band => ({ ...band }))} />
    <div className="stacked-share-plot">
      <TimeChart {...chart} layers={layers} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: 100, log: false }}
        formatValue={formatAxisPercent} valueTicks={SHARE_TICKS} />
    </div>
  </div>;
}
