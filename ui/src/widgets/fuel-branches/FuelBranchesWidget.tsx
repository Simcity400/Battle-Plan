import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { motionOf } from '../../charts/easing.ts';
import { jumpItem } from '../../charts/jump';
import type { PlaybackMode } from '../../charts/view';
import { isSlotKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import { readDatedCountry } from '../../dashboard/datedCountry.ts';
import GateState from '../../dashboard/GateState';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import type { Holder } from '../../dashboard/regionGate';
import { FOLLOW, recordedHour } from '../../dashboard/regionTime.ts';
import { coverageOf, useRegion, useRegionOutside } from '../../dashboard/widgetTime.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry.tsx';
import FuelConsumptionBreakdown from '../fuel/FuelConsumptionBreakdown';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from '../fuel/fuelConsumption';
import { fuelDecimals } from './fuelBranchesData';
import {
  branchDrillCanFollow,
  branchDrillDetailSubject,
  branchDrillSelectionKey,
  resolveBranchDrillOpen,
  type BranchFamilyLevel,
} from './fuelBranchesDrill';

const DETAIL_HOURS = 720;
const HOLDERS = { holders: true };
const DETAIL_TITLE = 'Fuel consumption by consumer';

export default function FuelBranchesWidget(props: FamilyViewProps) {
  const { widget, runtime, active, options } = props;
  const widgetRegion = useRegion(widget);
  const drillControl = useDrill(runtime, widgetRegion, null, undefined, undefined, HOLDERS);
  const storedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [drillOwner, setDrillOwner] = useState<object>();
  const [chartHolder, setChartHolder] = useState<Holder | null>(null);
  const handleCountryHolder = useCallback((next: Holder | null) => {
    setChartHolder(prev => prev?.tag === next?.tag && prev?.origin === next?.origin ? prev : next);
  }, []);
  const digits = fuelDecimals(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const drillOpen = storedDrill.breakdown !== null;
  const followRegion = useRegion(widget, FOLLOW);
  const playbackOutside = useRegionOutside(runtime, followRegion, active && drillOpen);
  const playbackCovered = !!track && playbackOutside === null;
  const playing = usePlaybackMotion(runtime.clock, active && drillOpen) !== undefined;
  const clockHour = usePlaybackHour(runtime.clock, active && drillOpen && drillControl.following);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const branchExtent: [number, number] = track ? [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : branchExtent) : [0, 1];
  const menu = useContextMenu('Fuel by branch actions');
  const anchor = useRef<HTMLDivElement>(null);

  const chosen = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const savedSelection = useMemo(() => {
    if (!track || !chosen || isSlotKey(chosen)) return null;
    const parsed = readDatedCountry(options.country, track.source.metadata.campaignKey, track.branch.id);
    return parsed?.tag === chosen ? parsed : null;
  }, [track, chosen, options.country]);
  const pickScope = branchDrillSelectionKey({
    sourceId: track?.source.id,
    revision: track?.source.revision,
    branchId: track?.branch.id,
    chosen,
    savedSelection,
    branchFirstHour: track?.branch.firstHour,
    resolvedHolder: chartHolder,
    drillHolder: storedDrill.country ?? null,
    extent,
  });
  const pickOwner = useMemo(() => ({ scope: pickScope }), [pickScope]);
  const drill = drillOwner === undefined || drillOwner === pickOwner ? storedDrill : CLOSED;
  const actualHour = track ? recordedHour(track, clockHour + track.axisOffset) : null;
  const drillCoverage = coverageOf(drillControl.outside);
  const at = drill.breakdown !== null ? drillControl.hour : null;
  const drillFollows = drillControl.following;
  const campaign = track?.source.metadata.campaignKey ?? '';
  const drillCountry = drillControl.gate.country;
  const identitySpans = drillCountry?.aliases ?? null;
  const drillTag = drillCountry?.tag ?? null;
  const detailSubject = branchDrillDetailSubject(drillTag, drill.country, drill.breakdown !== null);
  const canFollow = branchDrillCanFollow(playbackCovered, drill.country, drillTag);
  const [consumption, setConsumption] = useState<FuelConsumptionSelection | null>(null);
  const [level, setLevel] = useState<FuelConsumptionLevel | null>(null);
  const [initialLevel, setInitialLevel] = useState<BranchFamilyLevel | null>(null);
  useEffect(() => {
    dispatch({ type: 'close' });
    setDrillOwner(undefined);
    setConsumption(null);
    setLevel(null);
    setInitialLevel(null);
  }, [pickOwner]);

  const open = (action: InstalledAction, activeHolder?: Holder | null) => {
    if (!track) return;
    const holder = activeHolder ?? chartHolder;
    const resolved = resolveBranchDrillOpen({ active, holder, extent, hour: action.hour, subject: action.subject });
    if (!resolved) return;
    setConsumption(null);
    setLevel(null);
    setInitialLevel(resolved.initialLevel);
    setDrillOwner(pickOwner);
    drillControl.show('consumption', resolved.hour, resolved.holder);
  };

  if (!track) return <WidgetState kind="empty" icon="droplet" message="Load a campaign to see fuel." />;
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const drillStage = drillControl.gate.stage.stage;
  const actorState = drillCountry && drillStage !== 'open' && drillStage !== 'outside' && drillStage !== 'unbound'
    ? <GateState gate={drillControl.gate} loading="Loading country…" />
    : undefined;
  const drillExtent: [number, number] = drillFollows || (at !== null && (at < extent[0] || at > extent[1]))
    ? branchExtent
    : extent;
  const chart = {
    active: active && drill.breakdown !== null,
    pickOwner,
    extent: drillExtent,
    branchStart: track.branch.firstHour,
    playhead: drillFollows ? actualHour : at,
    playback,
    liveHead: drillFollows ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    afterPlayhead: options.afterPlayhead === true,
    windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond: motionOf(runtime.clock.getSnapshot()),
    within: drillCountry?.history?.spans ?? null,
    detailHours: DETAIL_HOURS,
    onPick: (hour: number) => { setDrillOwner(pickOwner); drillControl.pick(hour); },
    onContextMenu: (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]),
  };
  const breakdown = at === null
    ? drillStage === 'open' || drillStage === 'outside'
      ? <WidgetState kind="outside" read={drillCoverage} />
      : <GateState gate={drillControl.gate} loading="Loading country…" />
    : actorState ?? (drillTag
      ? <FuelConsumptionBreakdown
          runtime={runtime}
          gate={drillControl.gate}
          widget={widget}
          track={track}
          tag={drillTag}
          hour={drillFollows ? null : at}
          follows={drillFollows}
          digits={digits}
          campaign={campaign}
          aliases={identitySpans ?? undefined}
          history={drillCountry?.history ?? null}
          selection={consumption}
          onSelection={setConsumption}
          level={level}
          onLevel={next => { setInitialLevel(null); setLevel(next); }}
          initialLevel={initialLevel}
          {...chart}
        />
      : <p className="drill-empty">Country unavailable at this time.</p>);

  const detailDay = drillFollows && playing ? 'Following playback' : at === null ? '' : dayLabel(at);
  const closeDetail = () => { setInitialLevel(null); dispatch({ type: 'close' }); };
  return <div ref={anchor} style={{ height: '100%', minHeight: 0 }}>
    {props.manifestDeclaration
      ? <FamilyWidget {...props} family={props.manifestDeclaration.family} declaration={props.manifestDeclaration} onActivate={open} onCountryHolder={handleCountryHolder} />
      : null}
    <DetailWindowHost open={drill.breakdown !== null} subject={detailSubject} hour={at} title={DETAIL_TITLE} day={detailDay} anchor={anchor} onClose={closeDetail}>
      <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null} anchor={anchor} title={DETAIL_TITLE}
        day={detailDay} canFollow={canFollow} onClose={closeDetail}>
        {breakdown}
      </DrillHost>
    </DetailWindowHost>{menu.menu}</div>;
}
