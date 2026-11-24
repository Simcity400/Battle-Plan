import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { motionOf } from '../../charts/easing.ts';
import { jumpItem } from '../../charts/jump';
import LiveText, { liveTexts } from '../../charts/LiveText';
import { dayLabel } from '../../charts/draw';
import { useChartTokens } from '../../charts/useChartTokens';
import type { PlaybackMode } from '../../charts/view';
import CountryPicker from '../../components/CountryPicker';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { formatDateTime } from '../../dashboard/calendar';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { countryKey } from '../../dashboard/countryKey.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupNumbers } from '../../dashboard/groupsReply';
import { resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { NumericReply } from '../../dashboard/numericReply';
import { resolveOptions } from '../../dashboard/options';
import { useHeldEdges } from '../../dashboard/regionGate';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { coverageOf, useRegion } from '../../dashboard/widgetTime.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { parseChartInspect } from '../../families/time-chart/hitTest.ts';
import { chooseDrillCountry, drillCountryCoverage, drillCountrySpans, heldDrillCountry, readDrillCountry } from '../country-measure/drillCountry';
import { STABILITY_BREAKDOWNS, STABILITY_OPTIONS } from './definition';
import StabilityDrill, { MEASURE_NAMES } from './StabilityDrill';
import type { Measure } from './stabilityData';
import { chooseStabilityActor, pinStabilityDrill, stabilityPickAction, type StabilityPick } from './stabilityDrillData';
import './stability.css';

const HOLDERS = { holders: true };
const MEASURES: readonly Measure[] = ['stability', 'warSupport'];

function pickFromAction(subject: string, actionHour: number, tags: readonly string[], shown: readonly Measure[]): StabilityPick | null {
  const parsed = parseChartInspect(subject);
  if (!tags.length || !shown.length || !Number.isFinite(actionHour)) return null;
  if (parsed && parsed.kind !== 'plot' && parsed.kind !== 'line') return null;
  const hour = Math.floor(actionHour);
  if (!parsed) {
    const [first, second] = subject.split(':');
    const tag = second === undefined ? tags[0]! : first!;
    const measure = (second ?? first) as Measure;
    if (tags.includes(tag) && MEASURES.includes(measure)) {
      return { hour, candidates: [tag], lines: [{ tag, measure }], tag, measure };
    }
  }
  const lines = tags.flatMap(tag => shown.map(measure => ({ tag, measure })));
  return { hour, candidates: [...tags], lines, tag: tags.length === 1 ? tags[0]! : null, measure: shown[0]! };
}

export default function StabilityDetail({ widget, runtime, active, countrySlot, chart }: WidgetProps & {
  readonly chart?: (onActivate: (subject: string, hour: number) => void) => ReactNode;
}) {
  const tokens = useChartTokens();
  const options = resolveOptions(STABILITY_OPTIONS, widget.options);
  const pinnedMeasure = options.show === 'stability' || options.show === 'warSupport' ? options.show as Measure : null;
  const [heldEdges, holdEdges] = useHeldEdges();
  const drillNeeds = useMemo(() => pinnedMeasure ? { changes: heldEdges } : { ...HOLDERS, changes: heldEdges }, [heldEdges, pinnedMeasure]);
  const drillControl = useDrill(runtime, useRegion(widget), pinnedMeasure ?? null, undefined, undefined, drillNeeds);
  const drill = drillControl.state;
  const dispatchDrill = drillControl.dispatch;
  const [drillMeasure, setDrillMeasure] = useState<Measure>(pinnedMeasure ?? 'stability');
  useEffect(() => { if (pinnedMeasure) setDrillMeasure(pinnedMeasure); }, [pinnedMeasure]);
  const shown = useMemo(() => MEASURES.filter(measure => options[measure] !== false), [options.stability, options.warSupport]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const hasSavedCountry = Object.hasOwn(widget.options ?? {}, 'stabilityCountry');
  const bindingKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries, hasSavedCountry, widget.options?.stabilityCountry, pinnedMeasure]);
  const bindingOwner = useMemo(() => ({ client: runtime.client, key: bindingKey }), [runtime.client, bindingKey]);
  const following = widget.time.mode === 'follow';
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const groupStore = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const groupData = groupStore.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries]), groups.data, groupNumbers);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupData, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groupData, lastHour]);
  const single = resolved.tags.length === 1 ? resolved.tags[0] : null;
  const clockHour = usePlaybackHour(runtime.clock, active && drillControl.following);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const axisOffset = track?.axisOffset ?? 0;
  const clockHead = useMemo(() => ({ read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame }),
    [runtime.clock, axisOffset]);
  const dayText = useMemo(() => liveTexts(runtime.clock, () => Math.floor(clockHead.read()), hour => new Map([['day', dayLabel(hour)]])), [runtime.clock, clockHead]);

  const pickScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries, widget.time, extent, resolved.tags, options.show]);
  const pickOwner = useMemo(() => ({ scope: pickScope }), [pickScope]);
  const [actorPick, setActorPick] = useState<{ owner: unknown; pick: StabilityPick } | null>(null);
  const chosen = actorPick?.owner === pickOwner ? actorPick.pick : null;
  const drillOpen = drill.breakdown !== null && chosen !== null;
  const drillTag = chosen ? chosen.tag : single;
  const choosingActor = chosen !== null && chosen.tag === null;
  useEffect(() => { dispatchDrill({ type: 'close' }); setActorPick(null); }, [pickOwner]);
  const pinnedRegionAt = pinnedMeasure ? drillControl.hour : null;
  const origin = useRef<{ owner: typeof bindingOwner; hour: number } | null>(null);
  if (origin.current?.owner !== bindingOwner) origin.current = null;
  if (pinnedMeasure && pinnedRegionAt !== null && !origin.current) origin.current = { owner: bindingOwner, hour: pinnedRegionAt };
  useEffect(() => { if (pinnedMeasure) dispatchDrill({ type: 'close' }); }, [bindingOwner]);
  const savedCountry = useMemo(() => readDrillCountry(widget.options?.stabilityCountry, track, widget.countries), [widget.options?.stabilityCountry, track, widget.countries]);
  const choiceHour = chosen?.hour ?? origin.current?.hour ?? null;
  const selectedCountry = useMemo(() => savedCountry ?? (hasSavedCountry ? null
    : groupData && track && drillTag && choiceHour !== null ? chooseDrillCountry(groupData.countryIdentities, track, drillTag, choiceHour) : null),
  [savedCountry, hasSavedCountry, groupData, track, drillTag, choiceHour]);
  const actorSpans = useMemo(() => selectedCountry && groupData && track ? drillCountrySpans(groupData.countryIdentities, selectedCountry, track, resolved.membership) : null,
    [selectedCountry, groupData, track, resolved.membership]);
  useEffect(() => holdEdges(actorSpans), [actorSpans, holdEdges]);
  const liveHour = track ? Math.floor(clockHour) + track.axisOffset : null;
  const drillCoverage = coverageOf(drillControl.outside);
  const drillAt = pinnedMeasure ? pinnedRegionAt : drillOpen ? drillControl.hour : null;
  const drillFollows = pinnedMeasure ? drillControl.following : drillOpen && drill.follow;
  const drillPlayhead = pinnedMeasure ? pinnedRegionAt : drillOpen && drill.follow ? liveHour : playhead;
  const shownTag = actorSpans && drillAt !== null ? countryDrillTagAt(actorSpans, Math.floor(drillAt)) : null;
  const drillOwner = useMemo(() => selectedCountry && actorSpans ? { key: countryKey(selectedCountry.slot), spans: drillCountryCoverage(actorSpans) } : null,
    [selectedCountry, actorSpans]);
  const drillLabels = useCountryLabelsAt(runtime.client, track, (drillOpen || pinnedMeasure && savedCountry) && shownTag && drillAt !== null ? [{ tag: shownTag, hour: Math.floor(drillAt) }] : [], active);
  const actorReason = hasSavedCountry && !savedCountry ? 'Selected country unavailable.'
    : !groupData ? 'Loading country…' : !selectedCountry || !actorSpans || !shownTag ? 'Selected country unavailable.' : null;
  const menu = useContextMenu('Chart actions');
  const anchor = useRef<HTMLDivElement | null>(null);

  const onActivate = useCallback((subject: string, hour: number) => {
    const picked = pickFromAction(subject, hour, resolved.tags, shown);
    if (!picked) return;
    setActorPick({ owner: pickOwner, pick: picked });
    setDrillMeasure(picked.measure);
    dispatchDrill(stabilityPickAction(picked, drillOpen, drillControl.followsAt));
  }, [resolved.tags, shown, pickOwner, drillOpen, drillControl.followsAt, dispatchDrill]);

  if (!track) return <WidgetState kind="empty" icon="sliders" message="Load a campaign to see stability and war support." />;
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const drillBody = (measure: Measure, onMeasure: ((measure: Measure) => void) | undefined, onPick: (hour: number) => void) =>
    <StabilityDrill client={runtime.client} clock={runtime.clock} gate={drillControl.gate} source={track.source} branch={track.branch} lastHour={lastHour}
      owner={shownTag ? drillOwner : null} measure={measure} onMeasure={onMeasure} hour={drillAt} following={drillFollows} liveHead={drillFollows ? clockHead : undefined}
      playback={{ playhead: drillPlayhead, mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }}
      extent={extent} onPick={onPick} onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])}
      branchStart={track.branch.firstHour} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} tokens={tokens} />;
  const unresolved = (hasSavedCountry && !savedCountry) || (groupData && (!selectedCountry || !actorSpans))
    ? <WidgetState kind="unavailable" message={actorReason!} />
    : groups.error && !groupData ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
      : !groupData ? <WidgetState kind="loading" label="Loading what makes up the values…" /> : null;

  if (pinnedMeasure) {
    const body = !single ? resolved.known(track.branch.firstHour, track.branch.lastHour) ? <WidgetState kind="empty" message="Choose one country to see what makes up its values." />
      : <WidgetState kind="loading" label="Loading what makes up the values…" />
      : unresolved ?? drillBody(pinnedMeasure, undefined, drillControl.pick);
    const countryHeader = countrySlot ? createPortal(<DatedCountryChip hour={drillAt === null ? null : Math.floor(drillAt)}
      label={savedCountry && shownTag && drillAt !== null ? drillLabels.labelAt(Math.floor(drillAt))(shownTag) : null}
      reason={hasSavedCountry ? actorReason : 'Choose a country.'} />, countrySlot) : null;
    return <div className="stability is-pinned" ref={anchor}>
      {countryHeader}
      <DrillHost drill={drillControl} runtime={runtime} mode="inline" anchor={anchor} title={MEASURE_NAMES[pinnedMeasure]}
        day={drillAt === null ? '' : drillFollows ? <LiveText live={dayText} at="day" /> : formatDateTime(Math.floor(drillAt))} canFollow main={null}>{body}</DrillHost>
      {menu.menu}
    </div>;
  }

  const breakdownOf = STABILITY_BREAKDOWNS.find(item => item.id === drillMeasure) ?? STABILITY_BREAKDOWNS[0];
  const pinTimed = drillControl.gate.checkTime();
  const pinHeld = drillControl.gate.checkHold(selectedCountry && actorSpans ? { tag: selectedCountry.tag, origin: track.branch.firstHour + selectedCountry.originOffset } : null);
  const pinCountry = pinHeld.held;
  const pinTime = pinTimed.time;
  return <div className="stability" ref={anchor}>
    <DrillHost drill={drillControl} runtime={runtime} subject={drillOpen} mode="window" anchor={anchor}
      title={`${shownTag && drillAt !== null ? `${drillLabels.labelAt(Math.floor(drillAt))(shownTag).name}: ` : ''}What makes up ${MEASURE_NAMES[drillMeasure].toLowerCase()}`}
      day={drillAt == null ? '' : drillFollows ? <LiveText live={dayText} at="day" /> : dayLabel(Math.floor(drillAt))}
      canFollow={!choosingActor && drillTag !== null && liveHour !== null && liveHour >= track.branch.firstHour && liveHour <= track.branch.lastHour}
      onClose={() => { dispatchDrill({ type: 'close' }); setActorPick(null); }}
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={pinCountry && pinTime ? () => runtime.addWidgetBelow(widget.id,
        pinStabilityDrill({ ...widget, options: { ...widget.options, stabilityCountry: JSON.stringify(heldDrillCountry(pinCountry, track)) } }, breakdownOf, track.id, pinCountry.tag, pinTime)) : undefined}
      main={chart?.(onActivate) ?? null}>
      {choosingActor && chosen && <CountryPicker countries={track.source.metadata.countries} playerCountryId={null}
        selectedTag={null} source={track.source} client={runtime.client} allowedTags={chosen.candidates}
        flagAt={{ branchId: track.branch.id, hour: chosen.hour }} onChange={tag => {
          const next = chooseStabilityActor(chosen, tag);
          if (next === chosen) return;
          setActorPick({ owner: pickOwner, pick: next });
          setDrillMeasure(next.measure);
          dispatchDrill({ type: 'pick', hour: next.hour });
        }} />}
      {drillOpen && drillAt === null ? <WidgetState kind="outside" read={drillCoverage} />
        : choosingActor ? <p className="drill-empty">Choose a country from the drawn lines for this moment.</p>
          : unresolved ?? drillBody(drillMeasure, setDrillMeasure, hour => dispatchDrill({ type: 'pick', hour }))}
    </DrillHost>
    {menu.menu}
  </div>;
}
