import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { drillHour } from '../../charts/drill/drillState';
import { CLOSED_PARTS_RACE, pinPartsRace, reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';
import CountryPicker from '../../components/CountryPicker';
import SupplyBreakdown from './SupplyBreakdown';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import StackedShareChart from '../../charts/StackedShareChart';
import WindowedRows from '../../charts/WindowedRows';
import type { MovingRow } from '../../charts/rowMotion';
import { jumpItem } from '../../charts/jump';
import { clipToSpans, runAt } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { PlaybackMode } from '../../charts/view';
import { motionOf } from '../../charts/easing.ts';
import LiveText, { liveTexts } from '../../charts/LiveText';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { historyAt } from '../../dashboard/countryIdentity';
import { isSlotKey } from '../../dashboard/countryKey.ts';
import { countryCapitulationBadges, countryCapitulationRows } from '../../dashboard/countryCapitulations';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import DatedCountryChip, { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveCountries } from '../../dashboard/groups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { NumericReply } from '../../dashboard/numericReply';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { definition } from './definition';
import { supplyCountrySelection } from './SupplyCountryChip';
import { STATES, STATE_LABELS, supplyTooltip } from './supplyHealthData';
import { useSupplySeries } from './supplySeries';
import './supplyHealth.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useChangedState } from '../../dashboard/changedState.ts';

const NO_WARNINGS: string[] = [];

export default function SupplyHealthWidget({ widget, runtime, active, headerSlot, countrySlot = null, onDiagnostics }: WidgetProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const body = useRef<HTMLDivElement>(null);
  const [bodyHeight, setBodyHeight] = useState(0);
  const [visibleTags, setVisibleTags] = useChangedState<string[]>([]);
  useLayoutEffect(() => {
    const element = body.current;
    if (!element) return;
    const measure = () => setBodyHeight(element.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const options = resolveOptions(definition.options, widget.options);
  const pinned = options.show === 'breakdown';
  const [drill, dispatchDrill] = useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE);
  const following = widget.time.mode === 'follow';
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const drillFollows = drill.breakdown !== null && drill.follow;
  const [drillOrigin, setDrillOrigin] = useState<{ tag: string; hour: number } | null>(null);
  const [drillWarnings, setDrillWarnings] = useState<string[]>([]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const savedSelection = supplyCountrySelection(widget, track);
  const invalidSavedSelection = pinned && Object.hasOwn(widget.options ?? {}, 'drillOriginHour') && !savedSelection;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const stores = useMemo(() => ({ groups: new NumericReply<CountryGroupReply>(), groupWarnings: new NumericReply<string[]>() }), [runtime.client]);
  const sourceScope = JSON.stringify([track?.source.id, track?.branch.id]);
  const incomingGroups = useCountryGroups(runtime, track, active);
  const groupData = stores.groups.update(sourceScope, incomingGroups.data, groupNumbers);
  const groupWarnings = stores.groupWarnings.update(sourceScope, incomingGroups.data?.warnings, reply => reply);
  const groups = { ...incomingGroups, data: groupData };
  const fallback = defaultCountryKey(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const countries = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window
    ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const axisOffset = track?.axisOffset ?? 0;
  const clockHead = useMemo(() => ({ read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame }),
    [runtime.clock, axisOffset]);
  const liveHead = following ? clockHead : undefined;
  const dayText = useMemo(() => liveTexts(runtime.clock, () => Math.floor(clockHead.read()), hour => new Map([['day', formatDateTime(hour)]])),
    [runtime.clock, clockHead]);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const head = Math.floor(playhead ?? extent[1]);
  const identities = useMemo(() => new Map((groups.data?.countryIdentities ?? []).map(history => [history.tag, history.identities])), [groups.data]);
  const hasActor = useCallback((tag: string, hour: number) => {
    const at = Math.floor(hour);
    return runAt((isSlotKey(tag) ? historyAt(groups.data?.countryIdentities ?? [], tag, at)?.identities : identities.get(tag)) ?? [], at) !== null;
  }, [identities, groups.data]);
  const [tip, setTip] = useChangedState<{ tag: string; hour: number } | null>(null);
  const reads = useMemo(() => tip ? [tip] : [], [tip]);
  const supply = useSupplySeries(runtime.client, runtime.clock, track?.source, track?.branch.id, countries.tags, countries.membership ?? undefined, lastHour, active, reads,
    following, body, track?.axisOffset);
  useShownEachFrame(runtime.clock, () => supply.drawn ? following ? Math.min(Math.floor(clockHead.read()), lastHour) : head : null);
  const { error, retry } = supply;
  const resetKey = JSON.stringify([track?.source.id, track?.branch.id, widget.countries, widget.time,
    options.show, widget.options?.drillOriginHour]);
  useEffect(() => { dispatchDrill({ type: 'close' }); setDrillOrigin(null); }, [resetKey]);
  const choices = drill.choices?.key === resetKey ? drill.choices : null;
  const clockHour = track ? Math.floor(playbackHour) + track.axisOffset : null;
  const drillAt = Math.floor((drill.breakdown === null && choices === null ? head : drillHour(drill, clockHour)) ?? head);
  const drillPlayhead = drill.breakdown !== null && drill.follow ? clockHour : playhead;
  const follow = () => { if (active && track) dispatchDrill({ type: 'follow' }); };
  const drillTag = drill.breakdown;
  const originHour = drillOrigin?.tag === drillTag ? drillOrigin.hour : drillAt;
  const open = (hour: number, tag: string) => {
    if (!active || !hasActor(tag, hour)) return;
    setDrillOrigin({ tag, hour: Math.floor(hour) });
    dispatchDrill({ type: 'open', breakdown: tag, hour, playing: runtime.clock.getSnapshot().playing,
      playhead: following ? playhead : null });
  };
  useEffect(() => {
    if (!pinned || invalidSavedSelection || !window || drill.breakdown !== null || drill.choices !== null || !groups.data) return;
    if (savedSelection) {
      const { tag: fixed, hour: originHour } = savedSelection;
      if (!hasActor(fixed, originHour)) return;
      setDrillOrigin({ tag: fixed, hour: originHour });
      dispatchDrill({ type: 'open', breakdown: fixed, hour: head, playing: false, playhead: following ? playhead : null });
      return;
    }
    const actors = countries.tags.filter(tag => hasActor(tag, head)
      && (!countries.membership || runAt(countries.membership.get(tag) ?? [], head)));
    if (actors.length === 1) {
      const tag = actors[0];
      setDrillOrigin({ tag, hour: head });
      dispatchDrill({ type: 'open', breakdown: tag, hour: head, playing: false, playhead: following ? playhead : null });
    } else if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour: head });
  }, [pinned, invalidSavedSelection, !!window, drill, groups.data, head, playhead, countries.tags, countries.membership, savedSelection?.tag, savedSelection?.hour, following, resetKey]);
  const drillOpen = drill.breakdown !== null || choices !== null;
  const membershipKey = useMemo(() => JSON.stringify([...(countries.membership ?? [])]), [countries.membership]);
  const countryScope = JSON.stringify(widget.countries);
  const series = useMemo(() => countries.tags.filter(tag => !countries.membership || countries.membership.has(tag)).map(tag => {
    const membership = countries.membership?.get(tag);
    const inRange = clipToSpans(supply.sharesOf(tag), [{ firstHour: extent[0], lastHour: extent[1] }]);
    const drawing = membership ? clipToSpans(inRange, membership) : inRange;
    const live = following ? supply.liveShares(tag, runs => {
      const shown = clipToSpans(runs, [{ firstHour: extent[0], lastHour: extent[1] }]);
      return membership ? clipToSpans(shown, membership) : shown;
    }) : undefined;
    return { tag, membership, drawing, live, missing: supply.missing.get(tag) ?? [], hours: drawing.map(run => run.firstHour),
      pickOwner: { client: runtime.client, source: sourceScope, selection: countryScope, first: extent[0], last: extent[1], tag, drawing } };
  }), [countries.tags, supply.shares, supply.missing, supply.liveShares, following, membershipKey, extent[0], extent[1], runtime.client, sourceScope, countryScope]);
  const badgesByTag = useMemo(() => countryCapitulationBadges(groups.data, series.map(country => ({ tag: country.tag, membership: country.membership,
    identities: identities.get(country.tag), segments: country.drawing })), extent, groups.data?.countryIdentities),
  [groups.data, series, identities, extent[0], extent[1]]);
  const wanted = useMemo(() => [...visibleTags.map(tag => ({ tag, hour: head })), ...(tip && visibleTags.includes(tip.tag) ? [tip] : [])]
    .filter(({ tag, hour }) => hasActor(tag, hour)), [visibleTags, head, tip, hasActor]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  useEffect(() => { if (!active) setTip(null); }, [active]);
  useEffect(() => { setTip(null); }, [track?.source.id, track?.source.revision, track?.branch.id]);
  const menu = useContextMenu('Supply health actions');
  const colors = useMemo(() => [tokens.error, tokens.warning, tokens.positive], [tokens]);
  const bands = useMemo(() => STATES.map((state, index) => ({ id: state, label: STATE_LABELS[state],
    color: colors[index], alpha: state === 'inSupply' ? 0.85 : 1 })), [colors]);
  const items = useMemo(() => series.map(item => ({ key: item.tag, item })), [series]);
  const onVisibleRows = useCallback((rows: readonly MovingRow<(typeof series)[number]>[]) => {
    const tags = rows.filter(row => row.phase !== 'leaving').map(row => row.item.tag);
    setVisibleTags(previous => previous.length === tags.length && previous.every((tag, index) => tag === tags[index]) ? previous : tags);
  }, []);
  useEffect(() => {
    const diagnostics = [...(groupWarnings ?? []),
      ...(incomingGroups.error ? [`Countries and capitulation markers are unavailable: ${incomingGroups.error.message}`] : []), ...drillWarnings];
    onDiagnostics(diagnostics.length ? diagnostics : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [groupWarnings, incomingGroups.error, drillWarnings, onDiagnostics]);

  const problem = (() => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see supply health." />;
    if (supply.refused) return <WidgetState kind="unavailable" message={supply.refused} />;
    if ((widget.time.mode === 'pin' || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (invalidSavedSelection) return <WidgetState kind="unavailable" message="Selected country unavailable." />;
    if (groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (countries.top != null) return <WidgetState kind="empty" message="Choose countries or a group in settings." />;
    if (countries.reason) return <WidgetState kind="empty" message={countries.reason} />;
    if (error && !supply.drawn) return <WidgetState kind="error" failure={error} onRetry={retry} />;
    if (!groups.data) return <WidgetState kind="loading" label="Loading supply health…" />;
    return undefined;
  })();
  const state = problem ?? (track ? <>
      {error && <WidgetState kind="error" failure={error} onRetry={retry} />}
      {bodyHeight > 0 && <WindowedRows items={items} rowHeight={bodyHeight} label="Supply health by country"
        className="supply-health-rows" onVisibleRows={onVisibleRows}>
        {row => {
          const country = row.item;
          const badges = badgesByTag.get(country.tag);
          const name = labels.labelAt(head)(country.tag).name;
          const reason = country.missing.find(span => span.firstHour <= extent[1] && span.lastHour >= extent[0])?.reason;
          return <section className="supply-health-country-chart" tabIndex={0}
            aria-label={name} onKeyDown={event => { if (event.target !== event.currentTarget) event.stopPropagation(); }}>
            {series.length > 1 && <p className="supply-health-country">{name}</p>}
            <StackedShareChart label={`Supply health · ${name}`}
                onSummaryView={supply.onSummaryView} onNavigateHour={supply.onNavigateHour} keyboardHour={supply.keyboardHour} keyboardRevision={supply.keyboardRevision}
                runs={country.drawing} live={country.live} bands={bands} extent={extent} playhead={playhead} liveHead={liveHead}
                playback={options.playback as PlaybackMode} afterPlayhead={options.afterPlayhead === true}
                windowHours={Number(options.windowDays) * 24} hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
                branchStart={track.branch.firstHour} active={active && row.phase !== 'leaving' && (!drillOpen || options.drill === 'popout')} notRecorded={supply.notRecorded}
                keyHours={badges?.length ? [...new Set([...country.hours, ...badges.map(badge => badge.hour)])].sort((a, b) => a - b) : country.hours} badges={badges}
                revision={`${labels.revision}:${viewport}:${reason ?? ''}`}
                tooltip={(hour, _frame, hovered = []) => {
                  const at = Math.floor(hour);
                  if (at < extent[0] || at > extent[1]) return null;
                  const membership = countries.membership?.get(country.tag);
                  if (membership && !runAt(membership, at)) return null;
                  const actor = hasActor(country.tag, at);
                  const label = actor ? labels.labelAt(at)(country.tag) : { name: 'Unknown country' };
                  const tip = supplyTooltip(formatDateTime(at), label.name, actor ? label : undefined,
                    actor ? supply.at(country.tag, at) : { missing: 'No country held this tag at this time.' }, colors);
                  return fitTooltip([{ ...tip, rows: [...tip.rows, ...countryCapitulationRows(hovered, labels.labelAt)] }], viewport, { keep: hovered.length });
                }}
                onHover={hour => setTip(previous => hour == null
                  ? previous?.tag === country.tag ? null : previous
                  : previous?.tag === country.tag && previous.hour === Math.floor(hour) ? previous : { tag: country.tag, hour: Math.floor(hour) })}
                onPick={hour => open(hour, country.tag)} pickOwner={country.pickOwner}
                onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
            {supply.drawn && !country.drawing.length && <p className="drill-empty">{supply.divisions.get(country.tag) === 0 ? 'No divisions in this period.'
                : reason ?? 'Supply was not recorded for this period.'}</p>}
          </section>;
        }}
      </WindowedRows>}
      {menu.menu}
    </> : null);
  const hasOrigin = Object.hasOwn(widget.options ?? {}, 'drillOriginHour');
  const unchosen = !hasOrigin && drillTag === null;
  const neutralHeader = pinned && countrySlot && (problem !== undefined || choices !== null || unchosen)
    ? createPortal(unchosen ? <ChooseCountryChip /> : <DatedCountryChip label={null} hour={window ? drillAt : null}
      reason="Selected country unavailable." />, countrySlot) : null;
  const drillCountry = drillTag !== null && isSlotKey(drillTag) ? historyAt(groups.data?.countryIdentities ?? [], drillTag, originHour)?.tag ?? null : drillTag;
  const breakdown = <SupplyBreakdown runtime={runtime} track={track} active={active} options={options} extent={extent}
    countrySlot={pinned && problem === undefined && choices === null && !unchosen ? countrySlot : null}
    headerSelection={hasOrigin || drillCountry === null ? savedSelection : { tag: drillCountry, hour: originHour }}
    identities={groups.data?.countryIdentities ?? null}
    playhead={drillPlayhead} liveHead={drillFollows ? clockHead : undefined} dayText={drillFollows ? dayText : undefined}
    tag={drillCountry} hour={drillAt} originHour={originHour} following={drill.follow} pinned={pinned}
    onPick={hour => active && dispatchDrill({ type: 'pick', hour })} onFollow={follow} onWarnings={setDrillWarnings} />;
  const choice = choices && track ? <div className="parts-race-details">
    <p className="drill-empty">Choose a country for {formatDateTime(drillAt)}.</p>
    <CountryPicker countries={track.source.metadata.countries} allowedTags={choices.tags} selectedTag={null} playerCountryId={null}
      source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: drillAt }}
      onChange={tag => {
        if (!active || tag === null || !choices.tags.includes(tag)) return;
        setDrillOrigin({ tag, hour: drillAt });
        dispatchDrill({ type: 'choose', key: choices.key, tag, hour: drillAt });
      }} />
  </div> : breakdown;
  return <>
    {groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {neutralHeader}
    {headerSlot && createPortal(<small className="dash-widget-subtitle"> · share of divisions</small>, headerSlot)}
    <div ref={body} className="supply-health">{pinned ? problem ?? choice : <DrillHost
      mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={body} title="Supply classification"
      day={drillFollows ? <LiveText live={dayText} at="day" /> : formatDateTime(drillAt)} following={drill.follow} canFollow={choices === null && track !== undefined}
      returnFocus=".supply-health-country-chart" onClose={() => dispatchDrill({ type: 'close' })} onFollow={follow}
      onPin={track && drillCountry ? () => runtime.addWidgetBelow(widget.id, pinPartsRace(
        { ...widget, time: drill.follow ? { mode: 'follow' } : widget.time,
          options: { ...widget.options, drillOriginHour: originHour } }, track, drillCountry, drillAt, drill.follow,
        'Supply classification')) : undefined} main={state}>{choice}</DrillHost>}</div>
  </>;
}
