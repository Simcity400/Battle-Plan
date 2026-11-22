import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../draw';
import CountryRaceView from './CountryRaceView';
import type { RaceView } from './options';
import type { TimeChartProps } from '../TimeChart';
import { useChartTokens } from '../useChartTokens';
import { useHeadHour } from '../../widgets/army-size/headHour.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { OptionValue } from '../../dashboard/options';
import { followStore } from '../../dashboard/follow/store.ts';
import { useFollow } from '../../dashboard/follow/useFollow.ts';
import type { FollowEntry } from '../../dashboard/follow/store';
import { stickyLimit, useStickyMembers } from '../../dashboard/follow/stickyMembers.ts';
import { resolveWindow } from '../../dashboard/model';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useViewRead } from '../../dashboard/viewRead';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useFactionColours } from '../../dashboard/FactionColours';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { BoundTrack, GroupMembership, WidgetSpec } from '../../dashboard/model';
import DrillHost from '../drill/DrillHost';
import { CLOSED, drillHour, pinnedBreakdown, reduceDrill } from '../drill/drillState';
import type { DrillAction, DrillState } from '../drill/drillState';
import type { Live } from '../live';
import FactionMembers from './FactionMembers';
import { FACTION_MEMBERS } from './factionMembersOptions.ts';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf';
import { factionNameLabel } from '../../dashboard/factionNames';
import { validateFactionLines, validateFactionRosters } from '../../dashboard/territoryReplies.ts';
import type { FactionRostersReply, SeriesStateReply } from '../../generated/dashboard';
import { liveRace, rowsOf } from './measureRaceData.ts';
import type { GroupTotal } from './factionTotalsData';
import { answerShape, answerValues, factionEntries, factionNote, factionSummaries, fixedRosters, membersPushed, rosterRows, rosterTags, stateValues } from './factionTotalsData.ts';
import './countryRace.css';
import type { FactionLines } from '../../dashboard/territoryReplies';

export interface FactionTotalsViewProps extends WidgetProps {
  options: Readonly<Record<string, OptionValue>>;
  kind: 'territory' | 'country-factories';
  measure: string;
  label: string;
  measureLabel: string;
  note?: string;
  format: (value: number) => string;
  group?: GroupTotal;
  only?: string;
}


const NO_WARNINGS: string[] = [];
const PARAMS = {};
const DECIMALS = 0;

function validateStates(value: unknown): SeriesStateReply {
  const reply = value as SeriesStateReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.states) || typeof reply.decimals !== 'number') throw new Error('The member state reply is malformed.');
  return reply;
}

export default function FactionTotalsView({ widget, runtime, active, onDiagnostics, options, kind, measure, label, measureLabel, note, format, group, only }: FactionTotalsViewProps) {
  const shown = useShownHour();
  const view: RaceView = options.view === 'lines' ? 'lines' : 'race';
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const axisOffset = track?.axisOffset ?? 0;
  const slots = widget.countries.kind === 'factions' ? widget.countries.count : Number.MAX_SAFE_INTEGER;
  const window = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? window.lastHour : null;
  const membership = widget.countries.kind === 'factions' || widget.countries.kind === 'group' ? widget.countries.membership : 'hour';
  const fixed = membership !== 'hour' || !!group;
  const rosters = useHeadProduct<FactionRostersReply>(runtime, widget, 'faction-rosters', track ? PARAMS : null,
    { active: active && view === 'race' && !fixed, validate: validateFactionRosters, structure: rosterRows });
  const groups = useCountryGroups(runtime, track, active && view === 'race' && fixed && !group);
  const held = useMemo(() => fixed && !group && groups.data ? fixedRosters(groups.data, membership as 'end' | 'ever', extent[0], extent[1]) : undefined,
    [fixed, group, groups.data, membership, extent[0], extent[1]]);
  const moment = Math.floor((fixed ? null : rosters.hour) ?? playhead ?? extent[1]);
  const factionData = fixed ? held : rosters.data;
  const rosterData = useMemo(() => group ? group.rosterAt(moment) : only && factionData
    ? { ...factionData, factions: factionData.factions.filter(faction => faction.id === only) } : factionData, [group, moment, only, factionData]);
  const rosterOf = useCallback((hour: number) => group ? group.rosterAt(hour) : rosterData, [group, rosterData]);
  const members = useMemo(() => group ? [...new Set(group.members.map(member => member.tag))].sort() : rosterTags(rosterData), [group, rosterData]);
  const family = useMemo(() => ({ kind, measure }), [kind, measure]);
  const following = active && view === 'race' && (group ? widget.time.mode === 'follow' : fixed ? widget.time.mode === 'follow' && !!held : rosters.read === 'followed');
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const [root, setRoot] = useState<HTMLDivElement | null>(null);
  const target = useMemo(() => ({ current: root }), [root]);
  const liveRef = useRef<ReturnType<typeof liveRace> | null>(null);
  const named = useStickyMembers(JSON.stringify([track?.branch.id, kind, measure, following]), members, stickyLimit(members.length));
  const query = useMemo(() => following && track && named.length
    ? { product: 'series-state', params: { branchId: track.branch.id, family, reads: named.map(tag => ({ country: countryFromKey(tag), hour: moment })) } } : null,
  [following, track?.branch.id, family, named]);
  const memberWakes = useMemo(() => new Set<() => void>(), []);
  const followed = useFollow(store, query, target, () => { liveRef.current?.wake(); for (const wake of [...memberWakes]) wake(); }, answerShape);
  const answered = useRef<{ store: unknown; entry: FollowEntry } | null>(null);
  if (followed.entry && membersPushed(followed.status, followed.entry.answer)) answered.current = { store, entry: followed.entry };
  else if (followed.status !== 'pending' && followed.status !== 'followed') answered.current = null;
  const valuesEntry = followed.entry && membersPushed(followed.status, followed.entry.answer) ? followed.entry
    : answered.current?.store === store ? answered.current?.entry ?? null : null;
  const pushed = valuesEntry !== null;
  const refused = pushed && valuesEntry?.answer?.kind === 'notRecorded' ? valuesEntry.answer.reason : null;
  const asked = useProduct<SeriesStateReply>(runtime.client, track && view === 'race' && !pushed && members.length ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'series-state',
    params: { branchId: track.branch.id, family, reads: members.map(tag => ({ country: countryFromKey(tag), hour: moment })) } } : null, active, validateStates, track?.source);
  const live = useMemo(() => valuesEntry ? liveRace(runtime.clock, axisOffset,
    hour => { shown(hour); return factionEntries(rosterOf(hour), answerValues(valuesEntry.answer, named, DECIMALS), slots, hour, format).entries; },
    () => valuesEntry.version, !!group) : null,
  [valuesEntry, runtime.clock, axisOffset, rosterOf, named, slots, format, shown, group]);
  liveRef.current = live;
  useHeadHour(runtime.clock, !!live, () => liveRef.current ? rowsOf(liveRef.current.entries()) : '', memberWakes);
  const race = useMemo(() => live ? { entries: live.entries(), unknown: 0 } : factionEntries(rosterData, stateValues(asked.data, moment), slots, moment, format),
    [live, rosterData, asked.data, slots, moment, format]);
  const [chartView, setChartView] = useState<{ from: number; to: number; width: number } | null>(null);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    setChartView(before => before && before.from === next.from && before.to === next.to && before.width === next.width ? before
      : { from: next.from, to: next.to, width: next.width });
  }, []);
  const groupParams = group ? { members: group.members.map(member => ({ tag: member.tag, spans: member.spans })), unread: group.unread } : null;
  const lane = track && view === 'lines' ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, kind, measure, membership, groupParams]) : null;
  const lines = useViewRead<{ from: number; to: number; width: number }, FactionLines>(lane, chartView, JSON.stringify(chartView), active,
    (at, signal, publish, restart) => loadProduct(runtime.client, { sourceId: track!.source.id, revision: track!.source.revision, product: 'faction-lines',
      params: { branchId: track!.branch.id, kind, measure, from: Math.floor(at.from), to: Math.floor(at.to), width: at.width, ...(groupParams ?? { membership }) } },
    signal, validateFactionLines, track!.source, restart, publish), runtime.client);
  const looks = useFactionColours(runtime.client, track?.source);
  const factions = useMemo(() => new Map(looks.factions.map(faction => [faction.id, faction])), [looks.factions]);
  const entityLooks = useMemo(() => group ? { name: () => group.label, color: () => tokens.accent } : {
    name: (id: string, hour?: number) => { const faction = factions.get(id); return faction ? factionNameLabel(faction, hour, looks.displayNames.get(id)) : 'Unknown faction'; },
    color: (id: string) => looks.colours.get(id) ?? tokens.accent,
  }, [group, factions, looks.colours, looks.displayNames, tokens.accent]);
  const summaries = useMemo(() => factionSummaries(lines.data && only ? { ...lines.data, lines: lines.data.lines.filter(line => line.id === only) } : lines.data, slots),
    [lines.data, slots, only]);
  useEffect(() => { if (!live) shown(view === 'race' ? asked.data ? moment : null : lines.data ? Math.floor(chartView?.to ?? moment) : null); });
  const lineRows = useMemo(() => [...summaries.keys()].map(id => ({ tag: id, runs: [] })), [summaries]);
  const liveHead = useMemo(() => widget.time.mode === 'follow' && track
    ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
  [widget.time.mode, runtime.clock, track]);

  const diagnosticKey = JSON.stringify([...(view === 'race' ? rosterData?.warnings ?? [] : lines.data?.warnings ?? []), ...looks.warnings,
    ...(fixed && view === 'race' ? groups.data?.warnings ?? [] : []),
    ...(view === 'race' && race.unknown ? [`${race.unknown} factions' totals are not known at this hour.`] : []),
    ...(!fixed && rosters.reason ? [`${label} rosters are asked by the fallback clock: ${rosters.reason}`] : []),
    ...(followed.reason ? [`${label} member values are asked per hour: ${followed.reason}`] : [])]);
  useEffect(() => {
    const warnings = JSON.parse(diagnosticKey) as string[];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [diagnosticKey, onDiagnostics]);

  const error = view === 'race' ? (fixed ? groups.error : rosters.error) ?? (pushed ? undefined : asked.error) : lines.error;
  const pinnedMembers = options.show === 'members';
  const resetKey = JSON.stringify([track?.source.id, track?.branch.id, kind, measure, widget.countries]);
  const [drillState, setDrill] = useState<{ key: string; drill: DrillState }>({ key: resetKey, drill: CLOSED });
  useWhyCommit('FactionTotalsView', { rosters: rosters.data, rostersAt: rosters.at, groups: groups.data, members: followed, valuesEntry, live, named,
    asked: asked.data, looks, lines: lines.data, chartView, drill: drillState });
  const drill = drillState.key === resetKey ? drillState.drill : CLOSED;
  const dispatch = useCallback((action: DrillAction) => setDrill(before => ({ key: resetKey, drill: reduceDrill(before.key === resetKey ? before.drill : CLOSED, action) })), [resetKey]);
  const memberFaction = drill.breakdown ?? (pinnedMembers ? only ?? null : null);
  const allGroups = useCountryGroups(runtime, track, active && memberFaction !== null);
  const factionGroup = useMemo(() => allGroups.data?.factions.find(faction => faction.id === memberFaction) ?? null, [allGroups.data, memberFaction]);
  const membersFollow = pushed && (drill.breakdown == null || drill.follow);
  const membersHour = Math.floor(drill.breakdown != null ? drillHour(drill, moment) ?? moment : moment);
  const membersHead = useMemo<Live<ReadonlyMap<string, number>> | null>(() => membersFollow && valuesEntry ? {
    read: () => answerValues(valuesEntry.answer, named, DECIMALS),
    subscribe: wake => { memberWakes.add(wake); return () => { memberWakes.delete(wake); }; },
  } : null, [membersFollow, valuesEntry, named, memberWakes]);
  const anchor = useRef<HTMLDivElement | null>(null);
  const membersOf = (picked: boolean) => track ? <FactionMembers runtime={runtime} track={track} active={active} options={options} kind={kind} measure={measure}
    label={`${memberFaction ? entityLooks.name(memberFaction, membersHour) : label} · ${measureLabel}`} format={format} faction={factionGroup}
    extent={extent} playhead={playhead} hour={membersHour} head={membersHead} onPick={picked ? hour => dispatch({ type: 'pick', hour }) : undefined} /> : null;
  const state = !track ? <WidgetState kind="empty" icon="shield" message="Load a campaign to see its factions." />
    : (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window ? <WidgetState kind="unavailable" message="Choose another time in widget settings." />
      : error ? <WidgetState kind="error" failure={error} onRetry={view === 'race' ? () => { rosters.retry(); groups.retry(); asked.retry(); } : lines.retry} />
        : looks.loading && !group ? <WidgetState kind="loading" label="Loading factions…" />
          : view === 'race' && rosterData?.missing ? <WidgetState kind="unavailable" message={rosterData.missing} />
            : view === 'race' && (!rosterData || !live && members.length > 0 && !asked.data) ? <WidgetState kind="loading" label={`Loading ${label.toLowerCase()}…`} /> : undefined;
  if (pinnedMembers) return <div className="country-race" ref={element => { setRoot(element); rosters.root(element); }}>
    {!only ? <WidgetState kind="empty" icon="shield" message="Choose a faction group shown as one total in the widget settings." />
      : allGroups.error && !allGroups.data ? <WidgetState kind="error" failure={allGroups.error} onRetry={allGroups.retry} />
        : !allGroups.data ? <WidgetState kind="loading" label="Loading faction members…" /> : membersOf(false)}
  </div>;
  const openMembers = !group && track ? (hour: number, id?: string) => {
    if (id) dispatch({ type: 'open', breakdown: id, hour, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? moment : null });
  } : undefined;
  const raceView = (visible: boolean) => <CountryRaceView runtime={runtime} track={track} state={state} active={active && visible} options={options} view={view} extent={extent} playhead={playhead}
      onPick={openMembers}
      moment={moment} slots={Math.max(1, Math.min(slots, race.entries.length || 1))} ranked={race.entries} liveRanked={live ?? undefined} lines={lineRows} countryTags={[]}
      entityLooks={entityLooks} lineLegendLabel="Factions" lineSummaries={summaries.size ? summaries : undefined}
      onSummaryView={view === 'lines' ? onSummaryView : undefined} liveHead={liveHead}
      label={label} empty={refused ? `Faction totals were not recorded on ${dayLabel(moment)}: ${refused}` : `No faction totals known on ${dayLabel(moment)}.`} formatValue={format}
      labels={{ race: hour => `${label}, ${measureLabel}, ${dayLabel(hour)}`, lines: `${label} over time, ${measureLabel}`, actions: `${label} actions` }}
      presentation={{ countryLooks: false, exact: true }}
      tooltip={(id, hour, name) => {
        const entry = (live?.entries() ?? race.entries).find(item => item.tag === id);
        const notes = [factionNote(rosterData, id, false), note].filter(Boolean).join(' ');
        return [{ title: name, subtitle: `${dayLabel(hour)} · ${measureLabel}`,
          rows: [{ label: measureLabel, value: entry ? format(entry.value) : 'Not recorded' }], ...(notes ? { note: notes } : {}) }];
      }}
      lineTooltip={(hour, ids, _labelAt, colour) => [{ title: dayLabel(hour), subtitle: measureLabel,
        rows: ids.map(id => ({ label: entityLooks.name(id, hour), value: '', swatch: colour(id), short: true })),
        ...(lines.data?.bounds ? { note: factionNote(undefined, '', true)! } : {}) }]} />;
  if (group || !track) return <div className="country-race" ref={element => { setRoot(element); rosters.root(element); }}>{raceView(true)}</div>;
  const opened = drill.breakdown;
  return <div className="country-race" ref={element => { setRoot(element); rosters.root(element); anchor.current = element; }}>
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={opened != null} anchor={anchor}
      title={opened ? `${entityLooks.name(opened, membersHour)}: members` : 'Faction members'} day={dayLabel(membersHour)} following={drill.follow}
      canFollow={widget.time.mode === 'follow'} onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })}
      onPause={() => dispatch({ type: 'pick', hour: Math.floor(runtime.clock.getHour()) + axisOffset })}
      onPin={opened ? () => runtime.addWidgetBelow(widget.id, pinFactionMembers(widget, track, opened, membersHour, drill.follow && widget.time.mode === 'follow', membership)) : undefined}
      main={raceView(opened == null || options.drill === 'popout')}>
      {opened ? allGroups.error && !allGroups.data ? <WidgetState kind="error" failure={allGroups.error} onRetry={allGroups.retry} />
        : !allGroups.data ? <WidgetState kind="loading" label="Loading faction members…" /> : membersOf(true) : null}
    </DrillHost>
  </div>;
}

export function pinFactionMembers(widget: WidgetSpec, track: BoundTrack, id: string, hour: number, follow: boolean, membership: GroupMembership): WidgetSpec {
  const pinned = pinnedBreakdown(widget, FACTION_MEMBERS, track.id);
  const offset = Math.floor(hour) - track.branch.firstHour;
  return { ...pinned, countries: { kind: 'group', group: { type: 'faction', id }, aggregate: true, membership },
    time: follow ? { mode: 'follow' } : { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } },
    options: { ...pinned.options, view: 'race' } };
}
