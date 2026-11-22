import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { runAt } from '../../charts/series';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { bandMaxOf, changesBy, runState, startUnseen, stretchAt } from '../../charts/timeline/runState';
import { laneRequests } from '../../charts/timeline/laneRequests';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { ColourScope } from '../../dashboard/campaignSettings';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { UNNAMED_FACTION } from '../../dashboard/factionNames';
import { useFactionColours } from '../../dashboard/FactionColours';
import { worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useCountriesAt } from '../../dashboard/useCountriesAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour';
import { OPTIONS } from './definition';
import { LEADER_GLYPH, factionChangeHours, factionGaps, factionLabel, factionLanes, factionViews, laneCountries, leaderAt, leadershipParts, membersAt,
  membersHeld, tagAt } from './factionsData';
import type { FactionOrder, MemberOrder } from './factionsData';
import './factions.css';
import FactionDrill from './FactionDrill';
import { motionOf } from '../../charts/easing.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';

const NO_WARNINGS: string[] = [];

export default function FactionsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const followsClock = widget.time.mode === 'follow';
  const structure = useRef<(hour: number) => string>(() => '');
  const playbackHour = useHeadHour(clock, active && followsClock, hour => structure.current(hour));
  const options = resolveOptions(OPTIONS, widget.options);
  const viewport = useViewportHeight();
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, active);
  const factionColours = useFactionColours(runtime.client, track?.source);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const data = groups.data;
  const { colours, names } = factionColours;
  const views = useMemo(() => data ? factionViews(data, names, id => colours.get(id) ?? tokens.muted) : [], [data, names, colours, tokens.muted]);
  const axisOffset = track?.axisOffset ?? 0;
  const changes = useMemo(() => data ? factionChangeHours(data, views) : [], [data, views]);
  structure.current = hour => `${changesBy(changes, Math.floor(hour + axisOffset))}:${clock.getSnapshot().playing ? '' : hour}`;
  const liveHead = useMemo(() => followsClock && track ? { read: () => clock.getSnapshot().position + axisOffset,
    subscribe: (changed: () => void) => clock.subscribeFrame(changed) } : undefined, [followsClock, track, clock, axisOffset]);
  useShownEachFrame(clock, () => data && track ? liveHead ? recordedHour(track, liveHead.read()) : at : null);
  const dashboardTag = track ? defaultCountryKey(track) : null;
  const lastHour = track?.branch.lastHour ?? 0;
  const filter = useMemo(() => worldFilter(widget.countries, dashboardTag ? [dashboardTag] : [], data, at, lastHour), [widget.countries, dashboardTag, data, at, lastHour]);
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const [drill, setDrill] = useState<string | null>(null);
  useEffect(() => { setDrill(null); }, [track?.source.id, track?.branch.id]);
  const [named, setNamed] = useState<string[]>([]);
  const [sortNames, setSortNames] = useState<string[]>([]);
  const identityHistories = useMemo(() => new Map(data?.countryIdentities.map(history => [history.tag, history.identities]) ?? []), [data]);
  const segmentAt = useCallback((tag: string, hour: number) => runAt(identityHistories.get(tag) ?? [], hour)?.firstHour ?? null, [identityHistories]);
  const { country, revision } = useCountriesAt(runtime.client, track, named, at, active, sortNames, segmentAt, undefined,
    { clock, on: followsClock, hour: (clockHour: number) => track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null });
  const hover = useHoverLabels(runtime.client, track, active, country);
  const branchFirstHour = track?.branch.firstHour ?? 0;
  const unobserved = data?.unobserved.factions;
  const gaps = useMemo(() => data ? factionGaps(data) : [], [data]);
  const { lanes, info } = useMemo(() => factionLanes(views, at, {
    order: options.order as FactionOrder, members: options.members as MemberOrder, ended: options.ended === 'shown', countries: filter.tags, countryIds: filter.countryIds,
  }, { country, day: dayLabel }, gaps, branchFirstHour), [views, at, options.order, options.members, options.ended, filter.tags, filter.countryIds, country, gaps, branchFirstHour]);
  const requests = laneRequests(lanes, inView);
  const wantedKey = JSON.stringify(laneCountries(requests.looks, info, at));
  const sortNamesKey = JSON.stringify(laneCountries(requests.names, info, at));
  useEffect(() => { setNamed(JSON.parse(wantedKey) as string[]); }, [wantedKey]);
  useEffect(() => { setSortNames(JSON.parse(sortNamesKey) as string[]); }, [sortNamesKey]);
  const onLanesInView = useCallback((start: number, end: number) => setInView([start, end]), []);
  const menu = useContextMenu('Timeline actions');
  const bandMax = useMemo(() => bandMaxOf(views.map(view => view.counts)), [views]);

  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...factionColours.warnings];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, factionColours.warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its factions." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="outside" />;
  const failure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (failure && !data) return failure;
  if (!data) return <>{failure}<WidgetState kind="loading" label="Loading factions…" /></>;
  const opened = drill == null ? null : views.find(view => view.id === drill);
  if (opened) return <div className="factions-widget">
    <FactionDrill runtime={runtime} widget={widget} track={track} view={opened} names={data.factions.find(faction => faction.id === opened.id)?.names ?? []}
      at={at} active={active} gaps={gaps} liveHead={liveHead} onBack={() => setDrill(null)} /></div>;
  if (filter.reason) return <>{failure}<WidgetState kind="unavailable" message={filter.reason} /></>;
  if (!data.factions.length) return <>{failure}<WidgetState kind="empty" message={data.warnings.length ? 'Faction data unavailable.' : 'No factions recorded.'} /></>;
  if (!lanes.length) return <>{failure}<WidgetState kind="empty" message={filter.tags ? 'None of the chosen countries has been in a faction yet.' : 'No factions yet.'} /></>;

  const shownFactions = new Set(lanes.filter(lane => !lane.depth).map(lane => info.get(lane.id)!.faction.id));
  const legend = views.filter(view => shownFactions.has(view.id)).map(view => {
    const name = factionLabel(view, at);
    return {
      id: view.id, label: name, color: view.color,
      onColor: (hex: string) => { factionColours.setColour(view.id, hex, options.colourScope as ColourScope); },
    };
  });
  const day = (hour: number) => dayLabel(hour);
  const tooltip = (laneId: string, hour: number, full: boolean): TooltipContent | null => {
    const about = info.get(laneId);
    if (!about) return null;
    const view = about.faction;
    const roster = view.unknown;
    const factionName = view.nameAt(hour);
    const nameRows: TooltipRow[] = factionName.reason ? [{ label: 'Name', value: 'Name unavailable', tone: 'muted' }] : [];
    const named = hover.labelAt(hour);
    if (!about.member) {
      const leader = leaderAt(view.leaders, hour);
      const count = membersAt(view, hour);
      const life = stretchAt(view.lifetime, hour);
      const start = life?.firstHour;
      const state = runState(life ? [life] : [], at, gaps);
      const rows: TooltipRow[] = [
        { label: 'Leader', value: leader ? named(leader).name : 'Not recorded', countryFlag: leader ? named(leader) : undefined, tone: leader ? undefined : 'muted' },
        { label: 'Members', value: count == null ? 'Not recorded' : String(count), swatch: view.color },
        ...(start != null ? [{ label: 'Founded', value: startUnseen(start, branchFirstHour, gaps) ? `before ${day(start)}` : day(start) }] : []),
        ...(state.kind === 'ended' ? [{ label: 'Dissolved', value: state.lastSeen == null ? day(state.at) : `by ${day(state.at)}` }]
          : state.kind === 'unknown' ? [{ label: 'Last recorded', value: day(state.lastSeen) }] : []),
      ];
      const belonged = count == null ? [] : membersHeld(view, hour);
      hover.ask(hour, leader && !belonged.includes(leader) ? [leader, ...belonged] : belonged);
      const members = belonged
        .sort((a, b) => (a === leader ? -1 : b === leader ? 1 : named(a).name.localeCompare(named(b).name)));
      const memberRows: TooltipRow[] = members.map((tag, index) => ({ label: `${tag === leader ? `${LEADER_GLYPH} ` : ''}${named(tag).name}`, countryFlag: named(tag),
        indent: true, short: true, divider: index === 0 }));
      const content: TooltipContent = { title: factionName.name ?? UNNAMED_FACTION, subtitle: `Faction · ${day(hour)}`, rows: [...nameRows, ...rows, ...memberRows] };
      return full ? content : fitTooltip([content], viewport, { more: hidden => `${hidden} more ${hidden === 1 ? 'member' : 'members'}` });
    }
    const member = about.member;
    const tag = tagAt(view, member, hour);
    hover.ask(hour, tag === null ? [] : [tag]);
    const label = tag === null ? { name: 'Unknown country', flagReason: 'Country unavailable at this time.' } : named(tag);
    const run = member.runs.find(item => item.firstHour <= hour && hour <= item.lastHour);
    const led = run ? leadershipParts(run, view.leaders).find(part => part.firstHour <= hour && hour <= part.lastHour)?.leader ?? false : false;
    const unknownThen = roster.some(span => span.firstHour <= hour && hour <= span.lastHour);
    const stretch = stretchAt(member.stretches, hour);
    if (!stretch) return null;
    const state = runState([stretch], at, roster);
    const rows: TooltipRow[] = [
      { label: 'Faction', value: factionName.name ?? UNNAMED_FACTION, swatch: view.color },
      ...nameRows,
      { label: 'Role', value: run ? (led ? `${LEADER_GLYPH} Leader` : 'Member') : unknownThen ? 'Not recorded' : 'Not a member', tone: run ? undefined : 'muted' },
      { label: 'Joined', value: startUnseen(stretch.firstHour, branchFirstHour, roster) ? `before ${day(stretch.firstHour)}` : day(stretch.firstHour) },
      { label: 'Left', value: state.kind === 'ended' ? (state.lastSeen == null ? day(state.at) : `by ${day(state.at)}`)
        : state.kind === 'unknown' ? `not recorded after ${day(state.lastSeen)}` : 'Still a member' },
    ];
    return { title: label.name, countryFlag: label, subtitle: `Member · ${day(hour)}`, rows };
  };
  const description = `${dayLabel(at)}: ${lanes.filter(lane => !lane.depth).map(lane => {
    const about = info.get(lane.id)!;
    return `${factionLabel(about.faction, at)}, ${lane.label}`;
  }).join('. ')}.`;
  return <div className="factions-widget">
    {failure}
    { }
    {options.legend === true && <Legend label="Factions" items={legend} always />}
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Faction membership" extent={extent} playhead={playhead} liveHead={liveHead}
      labelHour={liveHead ? undefined : at} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={false} description={description}
      unobserved={unobserved} bandMax={bandMax} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active}
      revision={`${revision}:${hover.revision}:${views.length}`} onLanesInView={onLanesInView}
      tooltip={(lane, _span, hour, full) => tooltip(lane.id, hour, full === true)}
      onLaneActivate={lane => { const about = info.get(lane.id); if (about && !about.member) setDrill(about.faction.id); }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
