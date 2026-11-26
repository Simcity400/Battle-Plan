import { useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { dayLabel } from '../../charts/draw';
import FollowControl, { DrawerDayLabel } from '../../charts/drill/FollowControl.tsx';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { useComparisonCountries } from '../../dashboard/comparisonCountries';
import GateState from '../../dashboard/GateState';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { isMember } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import { FOLLOW, recordedHour } from '../../dashboard/regionTime.ts';
import { coverageOf, useRegion, useRegionOutside } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { lineChartSettings } from '../../families/time-chart/settings.ts';
import { BREAKDOWNS } from './definition';
import FuelBreakdown from './FuelBreakdown';
import FuelConsumptionBreakdown from './FuelConsumptionBreakdown';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from './fuelConsumption';
import { fuelDecimals } from './fuelData';
import { memberSpansForAliases, pausedChartCutoff, type FuelRecipientSelection } from './fuelDrill';
import './fuel.css';

const DETAIL_HOURS = 720;
const HOLDERS = { holders: true };
const DETAIL_TITLE = 'Fuel consumption and daily accounting';

export default function FuelWidget(props: FamilyViewProps) {
  const { widget, runtime, active, options } = props;
  const drillControl = useDrill(runtime, useRegion(widget), null, undefined, undefined, HOLDERS);
  const storedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [drillOwner, setDrillOwner] = useState<object>();
  const digits = fuelDecimals(options);
  const { track, countries } = useComparisonCountries(widget, runtime, active);
  const tags = countries.tags;
  const membership = countries.membership ?? undefined;
  const drillOpen = storedDrill.breakdown !== null;
  const followRegion = useRegion(widget, FOLLOW);
  const playbackOutside = useRegionOutside(runtime, followRegion, active && drillOpen);
  const playbackCovered = !!track && playbackOutside === null;
  const playing = usePlaybackMotion(runtime.clock, active && drillOpen) !== undefined;
  const clockHour = usePlaybackHour(runtime.clock, active && drillOpen && drillControl.following && !playing);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const branchExtent: [number, number] = track ? [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : branchExtent) : [0, 1];
  const anchor = useRef<HTMLDivElement>(null);

  const pickScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tags, extent]);
  const pickOwner = useMemo(() => ({ scope: pickScope }), [pickScope]);
  const drill = drillOwner === undefined || drillOwner === pickOwner ? storedDrill : CLOSED;
  const [measure, setMeasure] = useState<'gain' | 'burn' | 'consumption'>('consumption');
  const drillMeasure = (BREAKDOWNS.find(item => item.id === drill.breakdown)?.id ?? measure) as 'gain' | 'burn' | 'consumption';
  const actualHour = track ? recordedHour(track, clockHour + track.axisOffset) : null;
  const drillCoverage = coverageOf(drillControl.outside);
  const at = drill.breakdown !== null ? drillControl.hour : null;
  const drillFollows = drillControl.following;
  const campaign = track?.source.metadata.campaignKey ?? '';
  const drillCountry = drillControl.gate.country;
  const identitySpans = drillCountry?.aliases ?? null;
  const drillTag = drillCountry?.tag ?? null;
  const [recipient, setRecipient] = useState<FuelRecipientSelection | null>(null);
  const [consumption, setConsumption] = useState<FuelConsumptionSelection | null>(null);
  const [level, setLevel] = useState<FuelConsumptionLevel | null>(null);
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(undefined); setRecipient(null); setConsumption(null); setLevel(null); }, [pickOwner]);

  const open = (hour: number) => {
    const tag = tags[0];
    if (!active || !track || !tag) return;
    hour = Math.floor(hour);
    if (hour < extent[0] || hour > extent[1] || !isMember(membership?.get(tag), hour)) return;
    setRecipient(null); setConsumption(null); setLevel(null);
    setDrillOwner(pickOwner);
    drillControl.show(drillMeasure, hour, { tag, origin: hour });
  };

  if (!track) return <WidgetState kind="empty" icon="droplet" message="Load a campaign to see fuel." />;
  const drillStage = drillControl.gate.stage.stage;
  const actorState = drillCountry && drillStage !== 'open' && drillStage !== 'outside' && drillStage !== 'unbound' ? <GateState gate={drillControl.gate} loading="Loading country…" /> : undefined;
  const drillExtent: [number, number] = drillFollows || (at !== null && (at < extent[0] || at > extent[1])) ? branchExtent : extent;
  const parentPlayhead = window ? window.lastHour : null;
  const pausedPlayhead = pausedChartCutoff(track, parentPlayhead, at, extent);
  const shared = {
    active: active && drill.breakdown !== null,
    pickOwner,
    extent: drillExtent,
    playhead: drillFollows ? actualHour : pausedPlayhead,
    within: memberSpansForAliases(identitySpans, membership, drillTag),
    windowHours: lineChartSettings(options, drillFollows).windowHours,
    detailHours: DETAIL_HOURS,
    region: drillControl.region,
    onPick: (hour: number) => { setDrillOwner(pickOwner); drillControl.pick(hour); },
  };
  const breakdown = at === null ? drillStage === 'open' || drillStage === 'outside' ? <WidgetState kind="outside" read={drillCoverage} /> : <GateState gate={drillControl.gate} loading="Loading country…" />
    : actorState ?? (drillTag
    ? drillMeasure === 'consumption'
      ? <FuelConsumptionBreakdown runtime={runtime} gate={drillControl.gate} widget={widget} track={track} tag={drillTag} hour={drillFollows ? null : at} follows={drillFollows} digits={digits}
        campaign={campaign} aliases={identitySpans ?? undefined} history={drillCountry?.history ?? null} selection={consumption} onSelection={setConsumption} level={level} onLevel={setLevel} {...shared} />
      : <FuelBreakdown runtime={runtime} gate={drillControl.gate} widget={widget} track={track} tag={drillTag} measure={drillMeasure} hour={drillFollows ? null : at} follows={drillFollows}
        digits={digits} campaign={campaign} aliases={identitySpans ?? undefined} history={drillCountry?.history ?? null} selection={recipient} onSelection={setRecipient} {...shared} />
    : <p className="drill-empty">Country unavailable at this time.</p>);

  const detailSubject = drill.breakdown !== null
    ? (drill.country ? `${drill.country.tag}@${drill.country.origin}` : drillTag)
    : null;
  const canFollow = playbackCovered && (drill.country != null || !!drillTag);
  const detailDay = drillFollows
    ? (playing ? 'Following playback' : <DrawerDayLabel runtime={runtime} region={drillControl.region} active={active && drill.breakdown !== null} />)
    : at === null ? '' : dayLabel(at);
  const closeDetail = () => dispatch({ type: 'close' });
  return <div ref={anchor} className="fh-frame">
    {props.manifestDeclaration ? <FamilyWidget {...props} family={props.manifestDeclaration.family} declaration={props.manifestDeclaration} onActivate={action => open(action.hour)} /> : null}
    <DetailWindowHost
      open={drill.breakdown !== null}
      subject={detailSubject}
      hour={at}
      title={DETAIL_TITLE}
      day={detailDay}
      anchor={anchor}
      placement="beside"
      returnFocus=".fh-body"
      className="fuel-detail-window"
      headerActions={canFollow ? <FollowControl following={drillControl.following} onFollow={drillControl.follow} onPause={drillControl.pause} /> : undefined}
      onClose={closeDetail}
    >
      <div className="fuel-detail-body">
        <Segmented label="Breakdown of" value={drillMeasure} onChange={next => {
          setMeasure(next); setRecipient(null); setConsumption(null); setLevel(null);
          if (drill.breakdown !== null && at !== null) dispatch({ type: 'open', breakdown: next, hour: at, follow: drillControl.following, country: drill.country });
        }} options={BREAKDOWNS.map(item => ({ value: item.id, label: item.label }))} />
        {breakdown}
      </div>
    </DetailWindowHost>
  </div>;
}
