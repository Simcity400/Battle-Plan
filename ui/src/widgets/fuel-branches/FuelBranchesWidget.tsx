import { useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { motionOf } from '../../charts/easing.ts';
import { jumpItem } from '../../charts/jump';
import type { PlaybackMode } from '../../charts/view';
import { useComparisonCountries } from '../../dashboard/comparisonCountries';
import GateState from '../../dashboard/GateState';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import { recordedHour } from '../../dashboard/regionTime.ts';
import { coverageOf, useRegion } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry.tsx';
import FuelConsumptionBreakdown from '../fuel/FuelConsumptionBreakdown';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from '../fuel/fuelConsumption';
import { fuelDecimals } from './fuelBranchesData';
import { resolveBranchDrillOpen, type BranchFamilyLevel } from './fuelBranchesDrill';

const DETAIL_HOURS = 720;
const HOLDERS = { holders: true };
const DETAIL_TITLE = 'Fuel consumption by consumer';

export default function FuelBranchesWidget(props: FamilyViewProps) {
  const { widget, runtime, active, options } = props;
  const drillControl = useDrill(runtime, useRegion(widget), null, undefined, undefined, HOLDERS);
  const storedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [drillOwner, setDrillOwner] = useState<object>();
  const digits = fuelDecimals(options);
  const { track, countries } = useComparisonCountries(widget, runtime, active);
  const tags = countries.tags;
  const membership = countries.membership ?? undefined;
  const following = widget.time.mode === 'follow';
  const drillOpen = storedDrill.breakdown !== null;
  const playing = usePlaybackMotion(runtime.clock, active && drillOpen) !== undefined;
  const clockHour = usePlaybackHour(runtime.clock, active && drillOpen && drillControl.following);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const menu = useContextMenu('Fuel by branch actions');
  const anchor = useRef<HTMLDivElement>(null);

  const pickScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tags, extent]);
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

  const open = (hour: number, subject?: string) => {
    if (!track) return;
    const resolved = resolveBranchDrillOpen({ active, tag: tags[0], extent, membership, hour, subject });
    if (!resolved) return;
    setConsumption(null);
    setLevel(null);
    setInitialLevel(resolved.initialLevel);
    setDrillOwner(pickOwner);
    drillControl.show('consumption', resolved.hour, { tag: resolved.tag, origin: resolved.hour });
  };

  if (!track) return <WidgetState kind="empty" icon="droplet" message="Load a campaign to see fuel." />;
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const drillStage = drillControl.gate.stage.stage;
  const actorState = drillCountry && drillStage !== 'open' && drillStage !== 'outside' && drillStage !== 'unbound'
    ? <GateState gate={drillControl.gate} loading="Loading country…" />
    : undefined;
  const chart = {
    active: active && drill.breakdown !== null,
    pickOwner,
    extent,
    branchStart: track.branch.firstHour,
    playhead: drillFollows ? actualHour : playhead,
    playback,
    liveHead: drillFollows ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    afterPlayhead: options.afterPlayhead === true,
    windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond: motionOf(runtime.clock.getSnapshot()),
    within: drillTag ? membership?.get(drillTag) ?? null : null,
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
      ? <FamilyWidget {...props} family={props.manifestDeclaration.family} declaration={props.manifestDeclaration} onActivate={action => open(action.hour, action.subject)} />
      : null}
    <DetailWindowHost open={drill.breakdown !== null} subject={drillTag} hour={at} title={DETAIL_TITLE} day={detailDay} anchor={anchor} onClose={closeDetail}>
      <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null} anchor={anchor} title={DETAIL_TITLE}
        day={detailDay} canFollow={actualHour !== null && !!drillTag} onClose={closeDetail}>
        {breakdown}
      </DrillHost>
    </DetailWindowHost>{menu.menu}</div>;
}
