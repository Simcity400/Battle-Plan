import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import { jumpItem } from '../../charts/jump';
import WidgetState from '../../components/WidgetState';
import DrillParts from '../../charts/drill/DrillParts';
import StackedShareChart from '../../charts/StackedShareChart';
import { drawPlayhead } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import { clipToSpans, runAt } from '../../charts/series';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { fitTooltip } from '../../charts/tooltip';
import type { PlaybackMode } from '../../charts/view';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillSpans, countryDrillTagAt } from '../../dashboard/countryIdentity';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { NumericReply } from '../../dashboard/numericReply';
import { usePreparedSupply } from './preparedSupply';
import type { SupplyHealthReply } from '../../generated/dashboard';
import { STATES, STATE_LABELS, formatShare, supplyDrillHistories, supplyDrillNumbers, supplyDrillRuns, supplyShareRuns,
  supplyTooltip, totalDivisions } from './supplyHealthData';
import type { supplyCountrySelection } from './SupplyCountryChip';

export default function SupplyBreakdown({ runtime, track, active, options, countrySlot, headerSelection, extent, playhead, tag, hour, originHour,
  following, pinned, onPick, onFollow, onWarnings }: {
  runtime: WidgetRuntime; track: BoundTrack | undefined; active: boolean; options: Readonly<Record<string, OptionValue>>;
  countrySlot: HTMLElement | null; headerSelection: ReturnType<typeof supplyCountrySelection>;
  extent: [number, number]; playhead: number | null; tag: string | null; hour: number; originHour: number;
  following: boolean; pinned: boolean; onPick: (hour: number) => void; onFollow: () => void;
  onWarnings: (warnings: string[]) => void;
}) {
  const tokens = useChartTokens();
  const menu = useContextMenu('Supply classification actions');
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const [tipHour, setTipHour] = useState<number | null>(null);
  const incoming = usePreparedSupply(runtime.client, track, tag ? [tag] : [], hour, tipHour, active, originHour);
  const stores = useMemo(() => ({ numbers: new NumericReply<SupplyHealthReply>(), warnings: new NumericReply<string[]>() }), [runtime.client]);
  const sourceScope = JSON.stringify([track?.source.id, track?.branch.id, tag, originHour]);
  const data = stores.numbers.update(sourceScope, incoming.data, supplyDrillNumbers);
  const warnings = stores.warnings.update(sourceScope, incoming.data?.warnings, reply => reply);
  const { error, retry } = incoming;
  useEffect(() => { onWarnings(warnings ?? []); return () => onWarnings([]); }, [warnings, onWarnings]);
  const histories = useMemo(() => data ? supplyDrillHistories(data) : [], [data]);
  const identitySpans = useMemo(() => countryDrillSpans(histories, tag ?? '', originHour), [histories, tag, originHour]);
  const actor = countryDrillTagAt(identitySpans, hour);
  const runs = useMemo(() => data && tag ? clipToSpans(supplyDrillRuns(data, tag, originHour),
    [{ firstHour: extent[0], lastHour: extent[1] }]) : [], [data, tag, originHour, extent[0], extent[1]]);
  const extentKey = JSON.stringify(extent);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope: sourceScope, extent: extentKey, model: data }),
    [runtime.client, sourceScope, extentKey, data]);
  const shares = useMemo(() => supplyShareRuns(runs), [runs]);
  const tipActor = tipHour === null ? null : countryDrillTagAt(identitySpans, tipHour);
  const wanted = useMemo(() => [...(actor ? [{ tag: actor, hour }] : []),
    ...(tipActor && tipHour !== null ? [{ tag: tipActor, hour: tipHour }] : [])], [actor, hour, tipActor, tipHour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  const headerActor = !error && incoming.data && headerSelection?.tag === tag && headerSelection?.hour === originHour
    && runAt(incoming.data.series.find(series => series.tag === tag)?.identities ?? [], originHour) ? actor : null;
  const header = countrySlot && createPortal(<DatedCountryChip label={headerActor ? labels.labelAt(hour)(headerActor) : null} hour={hour}
    reason={error ? 'Couldn’t load the country.' : !headerActor ? 'Selected country unavailable.' : null} />, countrySlot);
  useEffect(() => { if (!active) setTipHour(null); }, [active]);
  const at = actor ? runAt(runs, hour) : null;
  const total = at ? totalDivisions(at) : null;
  const colors = useMemo(() => [tokens.error, tokens.warning, tokens.positive], [tokens.error, tokens.warning, tokens.positive]);
  const bands = useMemo(() => STATES.map((state, index) => ({ id: state, label: STATE_LABELS[state], color: colors[index],
    alpha: state === 'inSupply' ? 0.85 : 1 })), [colors]);
  const keyHours = useMemo(() => runs.map(run => run.firstHour), [runs]);
  if (!track || !tag) return <>{header}<WidgetState kind="empty" message="Choose a country for the supply classification." /></>;
  if (error) return <>{header}<WidgetState kind="error" detail={error} onRetry={retry} /></>;
  if (!data) return <>{header}<WidgetState kind="loading" label="Loading supply classification…" /></>;
  if (!data.available) return <>{header}<WidgetState kind="unavailable" message="Supply data is not in this recording." /></>;
  return <>{header}<div className="supply-health-breakdown">
    {!incoming.data && <WidgetState kind="loading" label="Refreshing supply classification…" />}
    {!(pinned && countrySlot && headerActor) && <p className="supply-health-country">{actor ? labels.labelAt(hour)(actor).name : 'Unknown country'}</p>}
    {pinned && <p className="drill-day">{formatDateTime(hour)} {track !== undefined && !following
      && <button type="button" className="button small" onClick={onFollow}>Follow playback</button>}</p>}
    <DrillParts parts={at ? STATES.map((state, index) => ({ id: state, label: STATE_LABELS[state], value: at[state],
      color: colors[index], text: usNumber.format(at[state]), shareText: formatShare(at[state], total!) })) : []}
      format={value => usNumber.format(value)} total={total ?? undefined}
      totalRow={{ label: 'Divisions', value: total === null ? 'Not known' : usNumber.format(total) }} />
    {(!at || total === 0) && <p className="drill-empty">{at ? 'No divisions.' : 'Supply was not recorded at this time.'}</p>}
    <StackedShareChart label="Supply classification history" runs={shares} summaries={actor ? incoming.summaries.get(actor) : undefined} bands={bands} extent={extent} playhead={playhead}
      playback={options.playback as PlaybackMode} afterPlayhead={options.afterPlayhead === true}
      windowHours={Number(options.windowDays) * 24} hoursPerSecond={hoursPerSecond} branchStart={track.branch.firstHour}
      onSummaryView={incoming.onSummaryView} onNavigateHour={incoming.onNavigateHour} keyboardHour={incoming.keyboardHour} keyboardRevision={incoming.keyboardRevision}
      active={active} keyHours={keyHours} revision={[data, hour, labels.revision]}
      overlay={(ctx, frame, chartTokens) => drawPlayhead(ctx, frame, chartTokens, hour)} onPick={onPick} pickOwner={pickOwner}
      onContextMenu={(atHour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, atHour)])}
      onHover={atHour => setTipHour(atHour === null ? null : Math.floor(atHour))}
      tooltip={atHour => {
        const at = Math.floor(atHour);
        const actor = countryDrillTagAt(identitySpans, at);
        const label = actor ? labels.labelAt(at)(actor) : { name: 'Unknown country', flag: undefined };
        return fitTooltip([supplyTooltip(formatDateTime(at), label.name, actor ? label : undefined,
          actor ? runAt(runs, at) : null, colors)], viewport);
      }} />
    {menu.menu}
  </div></>;
}
