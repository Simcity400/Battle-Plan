import { useMemo } from 'react';
import * as requests from '../../dashboard/batch5Requests';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import RunLines from '../../charts/RunLines';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { chartRuns } from '../../dashboard/keyedHistory';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentKey } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { measureField, LABELS, validateIntelCoverage } from './intelCoverageData';
import type { Basis, IntelCoverageReply, Measure } from './intelCoverageData';
import { useIntelOptions } from './useIntelCoverage';

/** One directed pair's chosen measure over the branch, held between reads. */
export default function PairHistory({ widget, runtime, active, pair, hour }: WidgetProps & { pair: { owner: string; target: string } | null; hour: number }) {
  const tokens = useChartTokens();
  const speed = usePlaybackMotion(runtime.clock);
  const menu = useContextMenu('Intel network history');
  const options = useIntelOptions(widget);
  const chosen = pair ?? (options.owner && options.target ? { owner: contentKey(String(options.owner)), target: contentKey(String(options.target)) } : null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track && chosen ? { sourceId: track.source.id, revision: track.source.revision, product: 'intel-network-coverage',
    params: requests.intelPair(track.branch.id, chosen.owner, chosen.target) } : null;
  const { data, error, retry } = useProduct<IntelCoverageReply>(runtime.client, request, active, validateIntelCoverage, track?.source);
  const field = measureField((options.measure as Measure) ?? 'covered', (options.basis as Basis) ?? 'controlled');
  const lines = useMemo(() => {
    const series = data?.window.series.find(item => item.name.endsWith(`|${field}`));
    return series ? [{ id: field, label: LABELS[field], color: tokens.accent, runs: chartRuns(series.runs, field.endsWith('Strength') ? 100000 : 1) }] : [];
  }, [data, field, tokens.accent]);
  if (!track) return <WidgetState kind="empty" icon="eye" message="Load a campaign to see intel networks." />;
  if (!chosen) return <WidgetState kind="empty" icon="eye" message="Choose a pair of countries: click a cell of the matrix." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the network's history…" />;
  return <div className="intel-pair-history">
    <RunLines label={`${LABELS[field]} over time`} extent={[track.branch.firstHour, track.branch.lastHour]} branchStart={track.branch.firstHour}
      playhead={null} hour={hour} lines={lines} known={data.window.known} active={active} revision={data} hoursPerSecond={speed}
      empty="No network" onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])} />
    {menu.menu}
  </div>;
}
