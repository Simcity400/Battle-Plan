import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { CLOSED, pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { motionOf } from '../../charts/easing.ts';
import { jumpItem } from '../../charts/jump';
import type { PlaybackMode } from '../../charts/view';
import { useComparisonCountries } from '../../dashboard/comparisonCountries';
import GateState from '../../dashboard/GateState';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import type { WidgetProps } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { isMember } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { readDatedCountry } from '../../dashboard/datedCountry.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';
import { coverageOf, useRegion } from '../../dashboard/widgetTime.ts';
import { DeclaredFamilyWidget } from '../../families/host/index.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { BREAKDOWNS, OPTIONS } from './definition';
import FuelBreakdown from './FuelBreakdown';
import FuelConsumptionBreakdown from './FuelConsumptionBreakdown';
import { fuelConsumptionLevelToken, fuelConsumptionToken, readFuelConsumption, readFuelConsumptionLevel } from './fuelConsumption';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from './fuelConsumption';
import { fuelDecimals } from './fuelData';
import { fuelExtentToken, fuelRecipientToken, readFuelExtent, readFuelRecipient } from './fuelDrill';
import type { FuelRecipientSelection } from './fuelDrill';
import { fuelDeclaration } from './fuelProjection.ts';

const DETAIL_HOURS = 720;
const HOLDERS = { holders: true };

export default function FuelWidget(props: WidgetProps) {
  const { widget, runtime, active, countrySlot } = props;
  const savedShow = BREAKDOWNS.find(item => item.id === widget.options?.show)?.id ?? null;
  const storedCountry = widget.options?.fuelCountry;
  const drillNeeds = useMemo(() => savedShow ? { country: { stored: storedCountry }, holders: true } : HOLDERS, [savedShow, storedCountry]);
  const drillControl = useDrill(runtime, useRegion(widget), savedShow, undefined, undefined, drillNeeds);
  const storedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [drillOwner, setDrillOwner] = useState<object>();
  const options = resolveOptions(OPTIONS, widget.options);
  const digits = fuelDecimals(options);
  const { track, countries } = useComparisonCountries(widget, runtime, active);
  const tags = countries.tags;
  const membership = countries.membership ?? undefined;
  const following = widget.time.mode === 'follow';
  const saved = BREAKDOWNS.find(item => item.id === options.show);
  const playing = usePlaybackMotion(runtime.clock) !== undefined;
  const clockHour = usePlaybackHour(runtime.clock, active && drillControl.following && !playing);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const heldExtent = track && saved ? readFuelExtent(options.fuelExtent, track.source.metadata.campaignKey, track.branch.id) : null;
  const extent: [number, number] = heldExtent ?? (track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1]);
  const menu = useContextMenu('Fuel actions');
  const anchor = useRef<HTMLDivElement>(null);

  const pickScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tags, options.show, extent]);
  const pickOwner = useMemo(() => ({ scope: pickScope }), [pickScope]);
  const drill = saved || drillOwner === undefined || drillOwner === pickOwner ? storedDrill : CLOSED;
  const [measure, setMeasure] = useState<'gain' | 'burn' | 'consumption'>('consumption');
  const drillMeasure = (saved?.id ?? BREAKDOWNS.find(item => item.id === drill.breakdown)?.id ?? measure) as 'gain' | 'burn' | 'consumption';
  const actualHour = track ? recordedHour(track, clockHour + track.axisOffset) : null;
  const drillCoverage = coverageOf(drillControl.outside);
  const at = drill.breakdown !== null ? drillControl.hour : null;
  const drillFollows = drillControl.following;
  const campaign = track?.source.metadata.campaignKey ?? '';
  const savedCountry = useMemo(() => track && saved ? readDatedCountry(options.fuelCountry, campaign, track.branch.id, widget.countries) : null,
    [track, saved, options.fuelCountry, widget.countries, campaign]);
  const drillCountry = drillControl.gate.country;
  const identitySpans = drillCountry?.aliases ?? null;
  const drillTag = drillCountry?.tag ?? null;
  const [recipient, setRecipient] = useState<FuelRecipientSelection | null | undefined>(undefined);
  const savedRecipient = track && saved?.id === 'burn' ? readFuelRecipient(options.fuelRecipient, campaign, track.branch.id) : null;
  const [consumption, setConsumption] = useState<FuelConsumptionSelection | null | undefined>(undefined);
  const savedConsumption = track && saved?.id === 'consumption' ? readFuelConsumption(options.fuelConsumption, campaign, track.branch.id) : null;
  const [level, setLevel] = useState<FuelConsumptionLevel | null | undefined>(undefined);
  const savedLevel = track && saved?.id === 'consumption' ? readFuelConsumptionLevel(options.fuelConsumptionLevel, campaign, track.branch.id) : null;
  const heldRecipient = recipient === undefined ? savedRecipient : recipient;
  const heldConsumption = consumption === undefined ? savedConsumption : consumption;
  const heldLevel = level === undefined ? savedLevel : level;
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(undefined); setRecipient(undefined); setConsumption(undefined); setLevel(undefined); }, [pickOwner]);
  const invalidSaved = !!saved && (!!options.fuelExtent && !heldExtent || !!options.fuelCountry && !savedCountry
    || saved.id === 'burn' && !!options.fuelRecipient && !savedRecipient || saved.id === 'consumption' && !!options.fuelConsumption && !savedConsumption
    || saved.id === 'consumption' && !!options.fuelConsumptionLevel && !savedLevel);

  const open = (hour: number) => {
    const tag = tags[0];
    if (!active || saved || !track || !tag) return;
    hour = Math.floor(hour);
    if (hour < extent[0] || hour > extent[1] || !isMember(membership?.get(tag), hour)) return;
    setRecipient(null); setConsumption(null); setLevel(null);
    setDrillOwner(pickOwner);
    drillControl.show(drillMeasure, hour, { tag, origin: hour });
  };

  if (!track) return <WidgetState kind="empty" icon="droplet" message="Load a campaign to see fuel." />;
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const drillStage = drillControl.gate.stage.stage;
  const actorState = drillCountry && drillStage !== 'open' && drillStage !== 'outside' && drillStage !== 'unbound' ? <GateState gate={drillControl.gate} loading="Loading country…" /> : undefined;
  const chart = { active: active && (!!saved || drill.breakdown !== null), pickOwner, extent, branchStart: track.branch.firstHour,
    playhead: drillFollows ? actualHour : playhead, playback,
    liveHead: drillFollows ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond: motionOf(runtime.clock.getSnapshot()), within: drillTag ? membership?.get(drillTag) ?? null : null, detailHours: DETAIL_HOURS,
    onPick: (hour: number) => { setDrillOwner(pickOwner); drillControl.pick(hour); },
    onContextMenu: (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]) };
  const breakdown = at === null ? drillStage === 'open' || drillStage === 'outside' ? <WidgetState kind="outside" read={drillCoverage} /> : <GateState gate={drillControl.gate} loading="Loading country…" />
    : invalidSaved ? <WidgetState kind="unavailable" message="Choose a country again." /> : actorState ?? (drillTag
    ? drillMeasure === 'consumption'
      ? <FuelConsumptionBreakdown runtime={runtime} gate={drillControl.gate} widget={widget} track={track} tag={drillTag} hour={drillFollows ? null : at} follows={drillFollows} digits={digits}
        campaign={campaign} aliases={identitySpans ?? undefined} history={drillCountry?.history ?? null} selection={heldConsumption} onSelection={setConsumption} level={heldLevel} onLevel={setLevel} {...chart} />
      : <FuelBreakdown runtime={runtime} gate={drillControl.gate} widget={widget} track={track} tag={drillTag} measure={drillMeasure} hour={drillFollows ? null : at} follows={drillFollows}
        digits={digits} campaign={campaign} aliases={identitySpans ?? undefined} history={drillCountry?.history ?? null} selection={heldRecipient} onSelection={setRecipient} {...chart} />
    : <p className="drill-empty">Country unavailable at this time.</p>);

  if (saved) {
    const headerReason = at === null ? 'This time is outside the recording.' : invalidSaved ? 'Choose a country again.' : !drillCountry?.historyHeld ? 'Loading country…'
      : !drillTag ? 'Country unavailable at this time.' : null;
    return <div ref={anchor} className="drill-body">
      <FuelPinnedHeader runtime={runtime} track={track} countrySlot={countrySlot} active={active} show={!!options.fuelCountry}
        tag={drillTag} hour={at} reason={headerReason} failure={drillCountry?.historyHeld ? undefined : drillCountry?.error} />
      {!drillFollows && <button type="button" className="button small quiet" onClick={() => {
        if (actorState || invalidSaved) return;
        setDrillOwner(pickOwner); drillControl.follow();
      }}>Follow playback</button>}
      {breakdown}{menu.menu}
    </div>;
  }

  const pinTimed = drillControl.gate.checkTime();
  const pinHeld = drillControl.gate.checkHold(drillCountry?.holder ?? null);
  const pinCountry = pinHeld.held;
  const pinTime = pinTimed.time;
  const detailTitle = 'Fuel consumption and daily accounting';
  const detailDay = drillFollows && playing ? 'Following playback' : at === null ? '' : dayLabel(at);
  const closeDetail = () => dispatch({ type: 'close' });
  return <div ref={anchor} style={{ height: '100%', minHeight: 0 }}>
    <DeclaredFamilyWidget {...props} adapter={timeChartHostAdapter} declaration={fuelDeclaration} onActivate={action => open(action.hour)} />
    <DetailWindowHost open={drill.breakdown !== null} subject={drillTag} hour={at} title={detailTitle} day={detailDay} anchor={anchor} onClose={closeDetail}>
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null} anchor={anchor} mode="inline" title={detailTitle}
      day={detailDay}
      canFollow={actualHour !== null && !!drillTag} onClose={closeDetail}
      main={null}
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={drillTag && !actorState && pinTime && pinCountry ? () => {
        const pinned = pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [pinCountry.tag] } }, BREAKDOWNS.find(item => item.id === drillMeasure)!, track.id, pinTime);
        pinned.options = { ...pinned.options, fuelCountry: pinCountry.token,
          fuelRecipient: heldRecipient ? fuelRecipientToken(heldRecipient) : '', fuelConsumption: heldConsumption ? fuelConsumptionToken(heldConsumption) : '',
          fuelConsumptionLevel: heldLevel ? fuelConsumptionLevelToken(heldLevel) : '', fuelExtent: fuelExtentToken(campaign, track.branch.id, extent) };
        runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>
      <Segmented label="Breakdown of" value={drillMeasure} onChange={next => {
        setMeasure(next); setRecipient(null); setConsumption(null); setLevel(null);
        if (drill.breakdown !== null && at !== null) dispatch({ type: 'open', breakdown: next, hour: at, follow: drillControl.following, country: drill.country });
      }} options={BREAKDOWNS.map(item => ({ value: item.id, label: item.label }))} />
      {breakdown}
    </DrillHost>
    </DetailWindowHost>{menu.menu}</div>;
}

function FuelPinnedHeader({ runtime, track, countrySlot, active, show, tag, hour, reason, failure }: {
  readonly runtime: WidgetProps['runtime'];
  readonly track: NonNullable<ReturnType<typeof useComparisonCountries>['track']>;
  readonly countrySlot: WidgetProps['countrySlot'];
  readonly active: boolean;
  readonly show: boolean;
  readonly tag: string | null;
  readonly hour: number | null;
  readonly reason: string | null;
  readonly failure: Error | undefined;
}) {
  const ready = show && !!countrySlot && active && reason === null && hour !== null && !!tag;
  const wanted = useMemo(() => ready && tag && hour !== null ? [{ tag, hour }] : [], [ready, tag, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, ready);
  if (!show || !countrySlot) return null;
  return createPortal(<DatedCountryChip failure={failure} label={ready && tag && hour !== null ? labelAt(hour)(tag) : null} hour={hour} reason={reason} />, countrySlot);
}
