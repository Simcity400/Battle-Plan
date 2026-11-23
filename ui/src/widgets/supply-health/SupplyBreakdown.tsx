import type { Gate } from '../../dashboard/regionGate';
import { failureText } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import { jumpItem } from '../../charts/jump';
import WidgetState from '../../components/WidgetState';
import DrillParts from '../../charts/drill/DrillParts';
import StackedShareChart from '../../charts/StackedShareChart';
import { drawPlayhead } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import { clipToSpans, runAt } from '../../charts/series';
import type { Span } from '../../charts/series';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { fitTooltip } from '../../charts/tooltip';
import type { Live } from '../../charts/live';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import type { PlaybackMode } from '../../charts/view';
import { motionOf } from '../../charts/easing.ts';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillSpans, countryDrillTagAt } from '../../dashboard/countryIdentity';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { followStore } from '../../dashboard/follow/store.ts';
import type { CountryIdentityHistory } from '../../generated/dashboard';
import { STATES, STATE_LABELS, supplyParts, supplyTooltip, totalDivisions } from './supplyHealthData.ts';
import { useSupplySeries } from './supplySeries';
import { useSupplyHead } from './supplyHead';
import type { supplyCountrySelection } from './SupplyCountryChip';
import { countryKey } from '../../dashboard/countryKey.ts';

export default function SupplyBreakdown({ runtime, gate, track, active, options, countrySlot, headerSelection, identities, extent, playhead, liveHead, dayText,
  tag, hour, originHour, following, pinned, onPick, onFollow, onWarnings }: {
  runtime: WidgetRuntime; gate: Gate; track: BoundTrack | undefined; active: boolean; options: Readonly<Record<string, OptionValue>>;
  countrySlot: HTMLElement | null; headerSelection: ReturnType<typeof supplyCountrySelection>;
  identities: readonly CountryIdentityHistory[] | null;
  extent: [number, number]; playhead: number | null;
  liveHead?: Live<number>; dayText?: LiveTexts;
  tag: string | null; hour: number; originHour: number;
  following: boolean; pinned: boolean; onPick: (hour: number) => void; onFollow: () => void;
  onWarnings: (warnings: string[]) => void;
}) {
  const tokens = useChartTokens();
  const menu = useContextMenu('Supply classification actions');
  const viewport = useViewportHeight();
  const root = useRef<HTMLDivElement>(null);
  const [tipHour, setTipHour] = useState<number | null>(null);
  const identitySpans = useMemo(() => identities && tag ? countryDrillSpans(identities, tag, originHour) : new Map<string, readonly Span[]>(),
    [identities, tag, originHour]);
  const slotKey = useMemo(() => {
    const identity = identities && tag ? runAt(identities.find(history => history.tag === tag)?.identities ?? [], originHour) : null;
    return identity ? countryKey(identity.slot) : null;
  }, [identities, tag, originHour]);
  const spans = useMemo(() => slotKey ? [...identitySpans.values()].flat().sort((a, b) => a.firstHour - b.firstHour) : identitySpans.get(tag ?? '') ?? [],
    [slotKey, identitySpans, tag]);
  const key = slotKey ?? tag;
  const tags = useMemo(() => key ? [key] : [], [key]);
  const within = useMemo(() => key ? new Map([[key, spans]]) : undefined, [key, spans]);
  const headAt = gate.recorded(hour, spans), tipAt = gate.recorded(tipHour, spans);
  const reads = useMemo(() => key ? [...(liveHead || headAt === null ? [] : [{ tag: key, hour: headAt }]), ...(tipAt === null ? [] : [{ tag: key, hour: tipAt }])] : [],
    [key, headAt, tipAt, liveHead]);
  const supply = useSupplySeries(runtime.client, runtime.clock, gate, track?.source, track?.branch, tags, within, track?.branch.lastHour ?? 0, active && !!identities, reads,
    liveHead !== undefined, root);
  const { error, retry } = supply;
  useEffect(() => { onWarnings(error ? [`Supply could not be loaded: ${error.message}`] : []); return () => onWarnings([]); }, [error, onWarnings]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const colors = useMemo(() => [tokens.error, tokens.warning, tokens.positive], [tokens.error, tokens.warning, tokens.positive]);
  const head = useSupplyHead(store, gate, liveHead !== undefined && active, track?.branch.id, key, root, colors);
  const actor = countryDrillTagAt(identitySpans, hour);
  const runs = useMemo(() => key ? clipToSpans(clipToSpans(supply.sharesOf(key), spans), [{ firstHour: extent[0], lastHour: extent[1] }]) : [],
    [supply.shares, key, spans, extent[0], extent[1]]);
  const live = useMemo(() => key && liveHead ? supply.liveShares(key, held => clipToSpans(clipToSpans(held, spans), [{ firstHour: extent[0], lastHour: extent[1] }])) : undefined,
    [supply.liveShares, key, liveHead, spans, extent[0], extent[1]]);
  const extentKey = JSON.stringify(extent);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope: JSON.stringify([track?.source.id, track?.branch.id, tag, originHour]),
    extent: extentKey, model: runs }), [runtime.client, track?.source.id, track?.branch.id, tag, originHour, extentKey, runs]);
  const tipActor = tipHour === null ? null : countryDrillTagAt(identitySpans, tipHour);
  const wanted = useMemo(() => [...(actor ? [{ tag: actor, hour }] : []),
    ...(tipActor && tipHour !== null ? [{ tag: tipActor, hour: tipHour }] : [])], [actor, hour, tipActor, tipHour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  const headerActor = identities && headerSelection?.tag === tag && headerSelection?.hour === originHour
    && runAt(identities.find(history => history.tag === tag)?.identities ?? [], originHour) ? actor : null;
  const header = countrySlot && createPortal(<DatedCountryChip failure={error} label={headerActor ? labels.labelAt(hour)(headerActor) : null} hour={hour}
    reason={error ? failureText(error, 'Couldn’t load the country.') : !headerActor ? 'Selected country unavailable.' : null} />, countrySlot);
  useEffect(() => { if (!active) setTipHour(null); }, [active]);
  const at = actor && key ? supply.at(key, hour) : { missing: 'No country held this tag at this time.' };
  const counts = at && 'counts' in at ? at.counts : null;
  const total = counts ? totalDivisions(counts) : null;
  const bands = useMemo(() => STATES.map((state, index) => ({ id: state, label: STATE_LABELS[state], color: colors[index],
    alpha: state === 'inSupply' ? 0.85 : 1 })), [colors]);
  const keyHours = useMemo(() => runs.map(run => run.firstHour), [runs]);
  if (!track || !tag) return <>{header}<WidgetState kind="empty" message="Choose a country for the supply classification." /></>;
  if (supply.refused) return <>{header}<WidgetState kind="unavailable" message={supply.refused} /></>;
  if (error && !supply.drawn) return <>{header}<WidgetState kind="error" failure={error} onRetry={retry} /></>;
  if (!identities) return <>{header}<WidgetState kind="loading" label="Loading supply classification…" /></>;
  return <>{header}<div className="supply-health-breakdown" ref={root}>
    {error && <WidgetState kind="error" failure={error} onRetry={retry} />}
    {!(pinned && countrySlot && headerActor) && <p className="supply-health-country">{actor ? labels.labelAt(hour)(actor).name : 'Unknown country'}</p>}
    {pinned && <p className="drill-day">{dayText ? <LiveText live={dayText} at="day" /> : formatDateTime(hour)} {!following
      && <button type="button" className="button small" onClick={onFollow}>Follow playback</button>}</p>}
    {head ? <DrillParts parts={[]} live={head} format={value => usNumber.format(value)} totalRow={{ label: 'Divisions', value: '' }} />
      : <>
        <DrillParts parts={counts ? supplyParts(counts, colors) : []}
          format={value => usNumber.format(value)} total={total ?? undefined}
          totalRow={{ label: 'Divisions', value: total === null ? at ? 'Not known' : 'Loading…' : usNumber.format(total) }} />
        {(!counts || total === 0) && <p className="drill-empty">{counts ? 'No divisions.' : at && 'missing' in at ? at.missing : 'Loading…'}</p>}
      </>}
    <StackedShareChart label="Supply classification history" runs={runs} live={live} bands={bands} extent={extent} playhead={playhead} liveHead={liveHead}
      playback={options.playback as PlaybackMode} afterPlayhead={options.afterPlayhead === true}
      windowHours={Number(options.windowDays) * 24} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
      onSummaryView={supply.onSummaryView} onNavigateHour={supply.onNavigateHour} keyboardHour={supply.keyboardHour} keyboardRevision={supply.keyboardRevision}
      active={active} notRecorded={supply.notRecorded} keyHours={keyHours} revision={[hour, labels.revision]}
      overlay={liveHead ? undefined : (ctx, frame, chartTokens) => drawPlayhead(ctx, frame, chartTokens, hour)} onPick={onPick} pickOwner={pickOwner}
      onContextMenu={(atHour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, atHour)])}
      onHover={atHour => setTipHour(atHour === null ? null : Math.floor(atHour))}
      tooltip={atHour => {
        const at = Math.floor(atHour);
        const actor = countryDrillTagAt(identitySpans, at);
        const label = actor ? labels.labelAt(at)(actor) : { name: 'Unknown country', flag: undefined };
        return fitTooltip([supplyTooltip(formatDateTime(at), label.name, actor ? label : undefined,
          actor && key ? supply.at(key, at) : { missing: 'No country held this tag at this time.' }, colors)], viewport);
      }} />
    {menu.menu}
  </div></>;
}
