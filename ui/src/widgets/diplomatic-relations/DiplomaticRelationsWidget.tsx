import { useCallback, useMemo, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { drawLineSummary } from '../../charts/chartSummary';
import { SECONDARY_DASH } from '../../charts/palette';
import TimeChart, { type ChartData, type ChartLayer } from '../../charts/TimeChart';
import { useChartTokens } from '../../charts/useChartTokens';
import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { useKeyedSeries } from '../../dashboard/keyedSeries';
import { keyId } from '../../dashboard/keyedSeriesData';
import { resolveOptions } from '../../dashboard/options';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import DeclaredHeatMatrixWidget from '../../families/heat/DeclaredHeatMatrixWidget.tsx';
import type { MatrixActivation } from '../../families/heat/index.ts';
import { OPTIONS, PAIR } from './definition.ts';
import { DIPLOMATIC_RELATIONS_DECLARATION, formatOpinion } from './diplomaticRelationsProjection.ts';

interface ChosenPair {
  readonly first: string;
  readonly firstId: number;
  readonly firstLabel: string;
  readonly second: string;
  readonly secondId: number;
  readonly secondLabel: string;
  readonly hour: number;
}

const OPINIONS = { kind: 'relation-opinions', measure: 'opinion' } as const;

function PairHistory({
  runtime,
  widget,
  active,
  pair,
}: WidgetProps & { readonly pair: ChosenPair }) {
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
  const hour = gate.hour;
  const following = widget.time.mode === 'follow';
  const root = useRef<HTMLDivElement>(null);
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const keys = useMemo(
    () => [
      { tag: pair.first, part: String(pair.secondId) },
      { tag: pair.second, part: String(pair.firstId) },
    ],
    [pair.first, pair.secondId, pair.second, pair.firstId],
  );
  const series = useKeyedSeries(
    runtime.client,
    runtime.clock,
    gate,
    track ? { source: track.source, branch: track.branch, family: OPINIONS, keys, partBits: 32, lastHour: track.branch.lastHour } : null,
    hour,
    gate.recorded(hoverHour),
    false,
    active,
    root,
  );
  const layers = useMemo<ChartLayer[]>(
    () => keys.map((key, index) => ({
      draw: (ctx, frame, chartTokens, state) => {
        const summary = series.summaryOf(keyId(key));
        if (summary) {
          drawLineSummary(
            ctx,
            frame,
            chartTokens,
            summary,
            index ? tokens.categories[1]! : tokens.categories[0]!,
            true,
            state.playhead ?? null,
            index ? { dash: SECONDARY_DASH } : {},
          );
        }
      },
    })),
    [keys, series.summaryOf, tokens.categories],
  );
  const chartData = useMemo<ChartData>(() => ({ revision: () => [series.revision()], subscribe: series.subscribe }), [series.revision, series.subscribe]);
  if (!track) return <WidgetState kind="empty" icon="handshake" message="Load a campaign to see diplomatic relations." />;
  if (series.error) return <WidgetState kind="error" failure={series.error} onRetry={series.retry} />;
  const valueAt = (index: number, at: number) => {
    const line = series.lines.find(item => keyId(item.key) === keyId(keys[index]!));
    const run = line?.runs.find(item => item.firstHour <= at && at <= item.lastHour);
    return run ? formatOpinion(Number(run.valueRaw)) : 'Not recorded';
  };
  const liveHead = following
    ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame }
    : undefined;
  return <div ref={root} style={{ width: '100%', height: '100%' }}>
    <TimeChart
      label={`${pair.firstLabel} and ${pair.secondLabel} opinions over time`}
      extent={[track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)]}
      playhead={hour}
      liveHead={liveHead}
      playback="cursor"
      branchStart={track.branch.firstHour}
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      layers={layers}
      active={active}
      notRecorded={series.unrecorded}
      revision={`${pair.first}:${pair.second}:${series.summaryByKey.size}`}
      data={chartData}
      onSummaryView={series.onSummaryView}
      onHover={setHoverHour}
      tooltip={at => ({
        title: dayLabel(at),
        rows: [
          { label: `${pair.firstLabel}’s opinion of ${pair.secondLabel}`, value: valueAt(0, Math.floor(at)), swatch: tokens.categories[0] },
          { label: `${pair.secondLabel}’s opinion of ${pair.firstLabel}`, value: valueAt(1, Math.floor(at)), swatch: tokens.categories[1] },
        ],
      })}
    />
  </div>;
}

export default function DiplomaticRelationsWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const [pair, setPair] = useState<ChosenPair | null>(null);

  const onActivate = useCallback((activation: MatrixActivation) => {
    if (activation.rowIndex === activation.columnIndex) return;
    const firstId = (activation.row.meta as { countryId?: number | null } | undefined)?.countryId;
    const secondId = (activation.column.meta as { countryId?: number | null } | undefined)?.countryId;
    if (typeof firstId !== 'number' || typeof secondId !== 'number') return;
    setPair({
      first: activation.row.id,
      firstId,
      firstLabel: activation.row.label,
      second: activation.column.id,
      secondId,
      secondLabel: activation.column.label,
      hour: activation.hour,
    });
    drillControl.dispatch({ type: 'open', breakdown: PAIR.id, hour: activation.hour, follow: drillControl.followsAt(activation.hour) });
  }, [drillControl]);

  if (options.show === PAIR.id) {
    if (!pair) return <WidgetState kind="empty" icon="handshake" message="Choose a pair of countries: click a cell of the matrix." />;
    return <PairHistory {...props} pair={pair} />;
  }

  const main = <DeclaredHeatMatrixWidget {...props} declaration={DIPLOMATIC_RELATIONS_DECLARATION} onActivate={onActivate} />;
  return <div ref={anchor} style={{ width: '100%', height: '100%' }}>
    <DrillHost
      drill={drillControl}
      runtime={runtime}
      anchor={anchor}
      mode={options.drill === 'popout' ? 'window' : 'inline'}
      title={pair ? `${pair.firstLabel} & ${pair.secondLabel}` : 'Pair history'}
      day={pair ? dayLabel(pair.hour) : ''}
      canFollow
      onClose={() => drillControl.dispatch({ type: 'close' })}
      main={main}
    >
      {pair && drillControl.state.breakdown !== null && active && <PairHistory {...props} pair={pair} />}
    </DrillHost>
  </div>;
}
