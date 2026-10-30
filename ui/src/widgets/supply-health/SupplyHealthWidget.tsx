import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { drillHour } from '../../charts/drill/drillState';
import { CLOSED_PARTS_RACE, pinPartsRace, reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';
import CountryPicker from '../../components/CountryPicker';
import SupplyBreakdown from './SupplyBreakdown';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import StackedShareChart from '../../charts/StackedShareChart';
import WindowedRows from '../../charts/WindowedRows';
import type { MovingRow } from '../../charts/rowMotion';
import { jumpItem } from '../../charts/jump';
import { clipToSpans, runAt } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { PlaybackMode } from '../../charts/view';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryCapitulationBadges, countryCapitulationRows } from '../../dashboard/countryCapitulations';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import DatedCountryChip, { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING, resolveCountries } from '../../dashboard/groups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { NumericReply } from '../../dashboard/numericReply';
import { usePreparedSupply } from './preparedSupply';
import { useParityRows } from '../../dashboard/parityRows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { SupplyHealthReply, SupplyHealthSeries, CountryGroupReply } from '../../generated/dashboard';
import { definition } from './definition';
import { supplyCountrySelection } from './SupplyCountryChip';
import { STATES, STATE_LABELS, supplyDrillNumbers, supplyShareRuns, supplyTooltip, totalDivisions } from './supplyHealthData';
import './supplyHealth.css';

const NO_WARNINGS: string[] = [];
const hasSupplyActor = (series: Pick<SupplyHealthSeries, 'identities' | 'runs'>, hour: number): boolean =>
  series.identities === undefined ? runAt(series.runs, hour) !== null : runAt(series.identities, hour) !== null;

export default function SupplyHealthWidget({ widget, runtime, active, headerSlot, countrySlot = null, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const body = useRef<HTMLDivElement>(null);
  const [bodyHeight, setBodyHeight] = useState(0);
  const [visibleTags, setVisibleTags] = useState<string[]>([]);
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
  const playbackHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const [drillOrigin, setDrillOrigin] = useState<{ tag: string; hour: number } | null>(null);
  const [drillWarnings, setDrillWarnings] = useState<string[]>([]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const savedSelection = supplyCountrySelection(widget, track);
  const invalidSavedSelection = pinned && Object.hasOwn(widget.options ?? {}, 'drillOriginHour') && !savedSelection;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const stores = useMemo(() => ({ groups: new NumericReply<CountryGroupReply>(), groupWarnings: new NumericReply<string[]>(),
    numbers: new NumericReply<SupplyHealthReply>(), warnings: new NumericReply<string[]>() }), [runtime.client]);
  const sourceScope = JSON.stringify([track?.source.id, track?.branch.id]);
  const incomingGroups = useCountryGroups(runtime, track, active && (widget.countries.kind === 'group' || !pinned));
  const groupData = stores.groups.update(sourceScope, incomingGroups.data, groupNumbers);
  const groupWarnings = stores.groupWarnings.update(sourceScope, incomingGroups.data?.warnings, reply => reply);
  const groups = { ...incomingGroups, data: groupData };
  const fallback = defaultCountryTag(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const countries = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window
    ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? window.lastHour : null;
  const head = Math.floor(playhead ?? extent[1]);
  const [tip, setTip] = useState<{ tag: string; hour: number } | null>(null);
  const incoming = usePreparedSupply(runtime.client, track, countries.tags, head, tip?.hour ?? null, active, undefined, visibleTags);
  const numericScope = JSON.stringify([sourceScope, widget.countries, countries.tags]);
  const data = stores.numbers.update(numericScope, incoming.data, supplyDrillNumbers);
  const warnings = stores.warnings.update(numericScope, incoming.data?.warnings, reply => reply);
  const { retry } = incoming;
  const error = useHeldFailure(incoming.error, incoming.data !== undefined, retry);
  useParityRows(body, () => {
    const rows = (incoming.data?.series ?? []).flatMap(series => series.runs.map(run => ({ tag: series.tag, ...run, identities: series.identities })));
    return { kind: 'windowed', label: 'Supply health exact counts', complete: incoming.parityComplete, total: rows.length,
      rows: rows.map((row, index) => ({ index, key: `${row.tag}:${row.firstHour}:${row.lastHour}`, ...row })),
      available: incoming.data?.available, reason: incoming.data?.reason, warnings: incoming.data?.warnings };
  }, { prepare: incoming.parityPrepare, ready: () => incoming.parityComplete, error: () => incoming.error ?? null });
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
    if (!active) return;
    const country = data?.series.find(series => series.tag === tag);
    if (!country || !hasSupplyActor(country, Math.floor(hour))) return;
    setDrillOrigin({ tag, hour: Math.floor(hour) });
    dispatchDrill({ type: 'open', breakdown: tag, hour, playing: runtime.clock.getSnapshot().playing,
      playhead: widget.time.mode === 'follow' ? playhead : null });
  };
  // A standalone multi-country breakdown asks for an actor instead of inventing one.
  useEffect(() => {
    if (!pinned || invalidSavedSelection || !window || drill.breakdown !== null || drill.choices !== null || !data) return;
    if (savedSelection) {
      const { tag: fixed, hour: originHour } = savedSelection;
      const country = data.series.find(series => series.tag === fixed);
      if (!country || !hasSupplyActor(country, originHour)) return;
      setDrillOrigin({ tag: fixed, hour: originHour });
      dispatchDrill({ type: 'open', breakdown: fixed, hour: head, playing: false,
        playhead: widget.time.mode === 'follow' ? playhead : null });
      return;
    }
    const actors = data.series.filter(series => hasSupplyActor(series, head)
      && (!countries.membership || runAt(countries.membership.get(series.tag) ?? [], head))).map(series => series.tag);
    if (actors.length === 1) {
      const tag = actors[0];
      setDrillOrigin({ tag, hour: head });
      dispatchDrill({ type: 'open', breakdown: tag, hour: head, playing: false,
        playhead: widget.time.mode === 'follow' ? playhead : null });
    } else if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour: head });
  }, [pinned, invalidSavedSelection, !!window, drill, data, head, playhead, countries.membership, savedSelection?.tag, savedSelection?.hour, widget.time.mode, resetKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const drillOpen = drill.breakdown !== null || choices !== null;
  const membershipKey = useMemo(() => JSON.stringify([...(countries.membership ?? [])]), [countries.membership]);
  const countryScope = JSON.stringify(widget.countries);
  const series = useMemo(() => (data?.series ?? []).filter(series => !countries.membership || countries.membership.has(series.tag)).map(series => {
    const inRange = clipToSpans(series.runs, [{ firstHour: extent[0], lastHour: extent[1] }]);
    const membership = countries.membership?.get(series.tag);
    const runs = membership ? clipToSpans(inRange, membership) : inRange;
    const geometry = clipToSpans(incoming.geometry.get(series.tag) ?? runs, [{ firstHour: extent[0], lastHour: extent[1] }]);
    const drawing = membership ? clipToSpans(geometry, membership) : geometry;
    return { drawing, tag: series.tag, identities: series.identities, membership, runs, shares: supplyShareRuns(drawing), hours: runs.map(run => run.firstHour),
      pickOwner: { client: runtime.client, source: sourceScope, selection: countryScope, first: extent[0], last: extent[1],
        tag: series.tag, runs, identities: series.identities, membership } };
  }), [data, incoming.geometry, membershipKey, extent[0], extent[1], runtime.client, sourceScope, countryScope]); // eslint-disable-line react-hooks/exhaustive-deps
  const seriesByTag = useMemo(() => new Map(series.map(item => [item.tag, item])), [series]);
  const badgesByTag = useMemo(() => countryCapitulationBadges(groups.data, series.map(country => ({ ...country, segments: country.runs })), extent),
    [groups.data, series, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const wanted = useMemo(() => [...visibleTags.map(tag => ({ tag, hour: Math.min(head, seriesByTag.get(tag)?.runs.at(-1)?.lastHour ?? head) })),
    ...(tip && visibleTags.includes(tip.tag) ? [tip] : [])].filter(({ tag, hour }) => {
      const country = seriesByTag.get(tag);
      return country && hasSupplyActor(country, hour);
    }), [visibleTags, head, tip, seriesByTag]);
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
    const diagnostics = [...(warnings ?? []), ...(data?.reason ? [data.reason] : []), ...(groupWarnings ?? []),
      ...(widget.countries.kind !== 'group' && !pinned && incomingGroups.error ? [`Capitulation markers are unavailable: ${incomingGroups.error}`] : []), ...drillWarnings];
    onDiagnostics(diagnostics.length ? diagnostics : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, warnings, groupWarnings, incomingGroups.error, widget.countries.kind, pinned, drillWarnings, onDiagnostics]);

  const problem = (() => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see supply health." />;
    if ((widget.time.mode === 'pin' || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (invalidSavedSelection) return <WidgetState kind="unavailable" message="Selected country unavailable." />;
    if (widget.countries.kind === 'group' && groups.error && !groups.data) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
    if (countries.top != null) return <WidgetState kind="empty" message="Choose countries or a group in settings." />;
    if (countries.reason) return <WidgetState kind="empty" message={countries.reason} />;
    if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
    if (!data || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label="Loading supply health…" />;
    if (!data.available) return <WidgetState kind="unavailable" message="Not in this recording" />;
    // A group's members at hours still being prepared are not known yet: no country is not proof of none.
    if (!series.length && !countries.known(extent[0], extent[1])) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
    return undefined;
  })();
  const state = problem ?? (track ? <>
      {!incoming.data && <WidgetState kind="loading" label="Refreshing supply health…" />}
      {bodyHeight > 0 && <WindowedRows items={items} rowHeight={bodyHeight} label="Supply health by country"
        className="supply-health-rows" onVisibleRows={onVisibleRows}>
        {row => {
          const country = row.item;
          const badges = badgesByTag.get(country.tag);
          const labelHour = Math.min(head, country.runs.at(-1)?.lastHour ?? head);
          return <section className="supply-health-country-chart" tabIndex={0}
            aria-label={labels.labelAt(labelHour)(country.tag).name} onKeyDown={event => { if (event.target !== event.currentTarget) event.stopPropagation(); }}>
            {series.length > 1 && <p className="supply-health-country">{labels.labelAt(labelHour)(country.tag).name}</p>}
            {!country.drawing.length ? countries.known(extent[0], extent[1]) ? <WidgetState kind="unavailable" message="Supply was not recorded for this period." />
              : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />
              : country.drawing.every(run => totalDivisions(run) === 0) ? <WidgetState kind="empty" message="No divisions in this period." />
              : <StackedShareChart label={`Supply health · ${labels.labelAt(labelHour)(country.tag).name}`}
                onSummaryView={incoming.onSummaryView} onNavigateHour={incoming.onNavigateHour} keyboardHour={incoming.keyboardHour} keyboardRevision={incoming.keyboardRevision}
                summaries={incoming.summaries.get(country.tag)} runs={country.shares} bands={bands} extent={extent} playhead={playhead}
                playback={options.playback as PlaybackMode} afterPlayhead={options.afterPlayhead === true}
                windowHours={Number(options.windowDays) * 24} hoursPerSecond={hoursPerSecond}
                branchStart={track.branch.firstHour} active={active && row.phase !== 'leaving' && (!drillOpen || options.drill === 'popout')}
                keyHours={badges?.length ? [...new Set([...country.hours, ...badges.map(badge => badge.hour)])].sort((a, b) => a - b) : country.hours} badges={badges}
                revision={`${head}:${labels.revision}:${viewport}`}
                tooltip={(hour, _frame, hovered = []) => {
                  const at = Math.floor(hour);
                  if (at < extent[0] || at > extent[1]) return null;
                  const membership = countries.membership?.get(country.tag);
                  if (membership && !runAt(membership, at)) return null;
                  const actor = hasSupplyActor(country, at);
                  const label = actor ? labels.labelAt(at)(country.tag) : { name: 'Unknown country' };
                  const tip = supplyTooltip(formatDateTime(at), label.name, actor ? label : undefined,
                    actor ? runAt(country.runs, at) : null, colors);
                  return fitTooltip([{ ...tip, rows: [...tip.rows, ...countryCapitulationRows(hovered, labels.labelAt)] }], viewport, { keep: hovered.length });
                }}
                onHover={hour => setTip(previous => hour == null
                  ? previous?.tag === country.tag ? null : previous
                  : previous?.tag === country.tag && previous.hour === hour ? previous : { tag: country.tag, hour })}
                onPick={hour => open(hour, country.tag)} pickOwner={country.pickOwner}
                onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />}
          </section>;
        }}
      </WindowedRows>}
      {menu.menu}
    </> : null);
  // Unpinned (a catalogue card), the header names the country the body opened or was given, once it has one.
  const hasOrigin = Object.hasOwn(widget.options ?? {}, 'drillOriginHour');
  const unchosen = !hasOrigin && drillTag === null;
  const neutralHeader = pinned && countrySlot && (problem !== undefined || choices !== null || unchosen)
    ? createPortal(unchosen ? <ChooseCountryChip /> : <DatedCountryChip label={null} hour={window ? drillAt : null}
      reason="Selected country unavailable." />, countrySlot) : null;
  const breakdown = <SupplyBreakdown runtime={runtime} track={track} active={active} options={options} extent={extent}
    countrySlot={pinned && problem === undefined && choices === null && !unchosen ? countrySlot : null}
    headerSelection={hasOrigin || drillTag === null ? savedSelection : { tag: drillTag, hour: originHour }}
    playhead={drillPlayhead} tag={drillTag} hour={drillAt} originHour={originHour} following={drill.follow} pinned={pinned}
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
    {widget.countries.kind === 'group' && groups.error && groups.data ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null}
    {neutralHeader}
    {headerSlot && createPortal(<small className="dash-widget-subtitle"> · share of divisions</small>, headerSlot)}
    <div ref={body} className="supply-health">{pinned ? problem ?? choice : <DrillHost
      mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={body} title="Supply classification"
      day={formatDateTime(drillAt)} following={drill.follow} canFollow={choices === null && track !== undefined}
      returnFocus=".supply-health-country-chart" onClose={() => dispatchDrill({ type: 'close' })} onFollow={follow}
      onPin={track && drillTag ? () => runtime.addWidgetBelow(widget.id, pinPartsRace(
        { ...widget, time: drill.follow ? { mode: 'follow' } : widget.time,
          options: { ...widget.options, drillOriginHour: originHour } }, track, drillTag, drillAt, drill.follow,
        'Supply classification')) : undefined} main={state}>{choice}</DrillHost>}</div>
  </>;
}
