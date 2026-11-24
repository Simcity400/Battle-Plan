import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { Live } from '../../charts/live';
import { laneRequests } from '../../charts/timeline/laneRequests';
import Timeline from '../../charts/timeline/Timeline';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useCountryFactsByKey } from '../../dashboard/countryNamesAt';
import { useFactionColours } from '../../dashboard/FactionColours';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { STYLES, useCountriesAt } from '../../dashboard/useCountriesAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { useShownEachFrame, useShownHour } from '../../dashboard/widgetPerf';
import type { CountryStyle } from '../../generated/dashboard';
import { mix } from '../../shell/appearance/color';
import { useHeadHour } from '../army-size/headHour';
import { OPTIONS } from './definition';
import type { WarContributionsReply, WarLossesReply, WarsReply } from '../../generated/dashboard';
import {
  NAME_WORDS, countryTagAt, labelObservationAt, laneCountries, lossesOf, observationAt, participantHoldingAt, prepareWars, retainExpandedWars,
  structureAt, structureHours, validateWarContributions, validateWarLosses, validateWarsReply, viewpointAt, viewpointSummary, warContributions,
  warsLanes, warsReplyState,
} from './warsData';
import type { CountryResolver, WarsColour, WarsLaneInfo, WarsView } from './warsData';
import { warTooltip } from './warsTooltip';
import { warsColourPlan, warsCountryColours, warsFixedColourKey, warsLookStart, warsSideColours } from './warsColours';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const pendingCountry = () => ({ name: NAME_WORDS.pending });
const GROWTH_STEPS = 64;

export default function WarsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const product = useProduct<WarsReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } }) : null, active, validateWarsReply, track?.source);
  const data = product.data;
  const groups = useCountryGroups(runtime, track, active);
  const factions = useFactionColours(runtime.client, track?.source);
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : undefined, [groups.data]);
  const prepared = useMemo(() => data && identities ? prepareWars(data, undefined, identities) : null, [data, identities]);
  const structure = useMemo(() => prepared ? structureHours(prepared) : [], [prepared]);
  const trackHour = useCallback((clockHour: number) => {
    const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
    return window ? Math.floor(window.lastHour) : null;
  }, [track, widget, runtime.tracks]);
  const branchFirst = track?.branch.firstHour ?? 0, branchLast = track?.branch.lastHour ?? 1;
  const growth = Math.max(24, Math.ceil((branchLast - branchFirst + 1) / GROWTH_STEPS));
  const grow = options.grow === true;
  const clockHour = useHeadHour(clock, active && following, hour => {
    const at = trackHour(hour);
    return at === null ? '' : `${structureAt(structure, at)}${grow ? `:${Math.ceil((at - branchFirst + 1) / growth)}` : ''}`;
  });
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : branchFirst;
  const last = widget.time.mode === 'range' && window ? window.lastHour : branchLast;
  const at = Math.floor(playhead ?? last);
  const growEnd = following ? Math.min(branchLast + 1, branchFirst + Math.ceil((at - branchFirst + 1) / growth) * growth) : at;
  const extent: [number, number] = [first, Math.max(first + 1, grow && playhead != null ? growEnd : last + 1)];
  const shown = useShownHour();
  useShownEachFrame(clock, () => following && data ? trackHour(clock.getHour()) ?? at : null);
  const liveHead = useMemo<Live<number> | undefined>(() => following ? {
    read: () => trackHour(clock.getHour()) ?? at,
    subscribe: changed => {
      const stops = [clock.subscribeFrame(changed), clock.subscribe(changed)];
      return () => { for (const stop of stops) stop(); };
    },
  } : undefined, [following, trackHour, clock, at]);
  useEffect(() => { if (!following) shown(data ? at : null); }, [following, data, at, shown]);
  const until = () => following ? trackHour(clock.getHour()) ?? at : at;
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const hoverAt = gate.recorded(hoverHour);
  const hourParams = track && hoverAt !== null ? { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag), hour: hoverAt } : null;
  const losses = useProduct<WarLossesReply>(runtime.client, track && hourParams ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'war-losses', params: hourParams }) : null, active, validateWarLosses, track?.source);
  const contributions = useProduct<WarContributionsReply>(runtime.client, track && hourParams ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'war-contributions', params: hourParams }) : null, active, validateWarContributions, track?.source);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => { setExpanded(new Set()); }, [track?.source.id, track?.branch.id, viewpointTag]);
  useEffect(() => {
    if (prepared) setExpanded(previous => retainExpandedWars(previous, prepared.byId));
  }, [prepared]);
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const [named, setNamed] = useState<string[]>([]);
  const namedSet = useMemo(() => new Set(named), [named]);
  const segmentAt = useCallback((tag: string, hour: number) => warsLookStart(prepared, track?.branch, tag, hour), [prepared, track?.branch]);
  const { country, revision } = useCountriesAt(runtime.client, track, named, at, active, named, segmentAt, NAME_WORDS, { clock, on: following, hour: trackHour });
  const looks = useCountryLooks(runtime.client, track, named, at, active, segmentAt);
  const fixed = tokens.countryColours === 'fixed';
  const stylePlan = useMemo(() => warsColourPlan(prepared, track?.branch, at, fixed), [prepared, track?.branch, at, fixed]);
  const styles = useCountryFactsByKey(runtime.client, track, stylePlan.wanted, active, STYLES, stylePlan.maxHours);
  const fixedColourKey = fixed ? warsFixedColourKey(stylePlan, styles) : null;
  const fixedColours = useMemo(() => {
    if (fixedColourKey == null) return new Map<string, string>();
    const inputs = JSON.parse(fixedColourKey) as [string, string | null, CountryStyle['color']][];
    const recorded = new Map(inputs.map(([key, , color]) => [key, color]));
    const corrected = warsCountryColours([...recorded.keys()], key => recorded.get(key) ?? null, tokens.surface, true);
    return new Map(inputs.flatMap(([key, tag]) => {
      const colour = corrected.get(key);
      return tag != null && colour !== undefined ? [[tag, colour] as const] : [];
    }));
  }, [fixedColourKey, tokens.surface]);
  const colours = useMemo(() => fixed ? fixedColours : warsCountryColours(named, tag => looks.color(tag, at), tokens.surface, false),
    [fixed, fixedColours, named, looks, at, tokens.surface]);
  const countries = useCallback<CountryResolver>((tag: string) => {
    if (track?.branch.gaps.some(gap => gap.firstHour <= at && at <= gap.lastHour)) return { name: NAME_WORDS.missing, known: false };
    return { ...country(tag), color: colours.get(tag) };
  }, [country, colours, track, at]);
  const neutral = mix(tokens.text, tokens.surface, 0.45);
  const sideColours = useMemo(() => prepared && options.view === 'countries' && options.colour === 'side'
    ? warsSideColours(prepared, groups.data, at, factions.colours) : undefined,
  [prepared, options.view, options.colour, groups.data, at, factions.colours]);
  const { lanes, info } = useMemo(() => prepared ? warsLanes(prepared, at, {
    view: options.view as WarsView, colour: options.colour as WarsColour, expanded,
    palette: tokens.categories, surface: tokens.surface, country: countries, named: namedSet, neutral, sideColours,
  }) : { lanes: [], info: new Map<string, WarsLaneInfo>() }, [prepared, at, options.view, options.colour, expanded, tokens.categories, tokens.surface, countries, namedSet, neutral, sideColours]);
  const viewpoint = data ? viewpointAt(data, at) : null;
  const requests = laneRequests(lanes, inView);
  const wantedKey = JSON.stringify([...new Set([...laneCountries(requests.looks, info, at), ...(viewpoint ? [viewpoint] : [])])]);
  useEffect(() => { setNamed(JSON.parse(wantedKey) as string[]); }, [wantedKey]);
  const onLanesInView = useCallback((start: number, end: number) => setInView([start, end]), []);
  const activate = useCallback((lane: TimelineLane) => {
    const about = info.get(lane.id);
    if (about?.kind !== 'war') return;
    setExpanded(previous => {
      const next = new Set(previous);
      if (next.has(about.war.id)) next.delete(about.war.id); else next.add(about.war.id);
      return next;
    });
  }, [info]);
  const hover = useHoverLabels(runtime.client, track, active, pendingCountry, NAME_WORDS.missing);
  const menu = useContextMenu('Timeline actions');
  useEffect(() => {
    const warnings = [...new Set([...(data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...factions.warnings,
      ...(factions.error ? [factions.error] : []), ...(groups.error ? [`Country history unavailable: ${groups.error.message}`] : []),
      ...(losses.error ? [`War casualty values unavailable: ${losses.error.message}`] : []),
      ...(contributions.error ? [`War contributions unavailable: ${contributions.error.message}`] : []),
      ...(prepared?.unidentifiedHours ? [`${prepared.unidentifiedHours} participant-hours have unknown countries; those lanes keep unknown labels and colours.`] : [])])];
    onDiagnostics(warnings.length ? warnings.map(warning => warning instanceof Error ? warning.message : warning) : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, factions.warnings, factions.error, losses.error, contributions.error, prepared, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its wars." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="outside" />;
  const state = warsReplyState(data, product.error);
  if (state === 'error') return <WidgetState kind="error" failure={product.error!} onRetry={product.retry} />;
  if (state === 'unavailable') return <WidgetState kind="unavailable" message="Wars are not in this recording." />;
  const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !prepared) return <>{groupFailure}<WidgetState kind="loading" label="Loading wars…" /></>;

  const lossesAt = (hour: number) => losses.data?.hour === Math.floor(hour) ? losses.data : undefined;
  const contributionsAt = (hour: number) => contributions.data?.hour === Math.floor(hour) ? contributions.data : undefined;
  const tooltip = (lane: TimelineLane, span: TimelineSpan | null, hour: number, full = false): TooltipContent | null => {
    const about = info.get(lane.id);
    if (!about) return null;
    const gap = track.branch.gaps.find(gap => gap.firstHour <= hour && hour <= gap.lastHour);
    const recorded = track.branch.firstHour <= hour && hour <= track.branch.lastHour && !gap;
    const datedLabels = recorded ? hover.labelAt(hour) : () => ({ name: NAME_WORDS.missing, known: false });
    const labels = (tag: string) => prepared.identities?.keyAt(tag, hour) === null
      ? { name: NAME_WORDS.missing, known: false } : datedLabels(tag);
    const bar = span ? about.spansById.get(span.id) : undefined;
    const war = about.kind === 'country' ? bar?.war : about.war;
    if (!war) {
      if (about.kind !== 'country') return null;
      const tag = countryTagAt(about.country, hour);
      if (recorded && tag !== null) hover.ask(hour, [tag]);
      return { ...countryNameDetails(tag === null ? null : labels(tag)), title: tag === null ? NAME_WORDS.missing : labels(tag).name, subtitle: dayLabel(hour), rows: [{ label: 'War', value: data.unobserved.some(gap => gap.firstHour <= hour && hour <= gap.lastHour)
        ? 'Not recorded' : 'No war' }] };
    }
    const observation = observationAt(war, hour);
    const labelled = labelObservationAt(war, hour);
    const tag = bar?.run?.tag ?? (about.kind === 'participant' ? countryTagAt(about.participant, hour)
      : about.kind === 'country' ? countryTagAt(about.country, hour) : null);
    const tags = tag ? [tag, ...(labelled ? [labelled.viewpointTag] : [])]
      : [...new Set([...(labelled ? [labelled.viewpointTag] : []), ...(observation ? [...observation.viewpointSide, ...observation.opposingSide] : [])])];
    if (recorded) hover.ask(hour, tags.filter(tag => prepared.identities?.keyAt(tag, hour) !== null));
    const participant = about.kind === 'participant' ? about.participant : bar?.participant;
    const heldLosses = lossesAt(hour);
    const warLosses = lossesOf(heldLosses, war.id);
    const heldContributions = contributionsAt(hour);
    const content = warTooltip({ war, observation, losses: warLosses?.run ?? null, lossesReason: warLosses?.reason ?? null,
      lossesPending: !heldLosses,
      contributions: warContributions(heldContributions?.contributions, observation),
      contributionsPending: !heldContributions,
      hour, until: until(), branchLast: track.branch.lastHour, unobserved: data.unobserved, branchGaps: track.branch.gaps, capitulationsUnobserved: data.capitulationsUnobserved,
      country: labels, day: dayLabel,
      ...(tag ? { participant: { tag, run: bar?.holding ?? (participant ? participantHoldingAt(participant, hour) : null),
        capitulations: participant?.capitulations } } : {}),
    });
    return full ? content : fitTooltip([content], viewport, { more: hidden => `${hidden} more ${hidden === 1 ? 'participant' : 'participants'}` });
  };
  const seenAs = viewpoint ? countries(viewpoint) : null;
  const summary = viewpointSummary(seenAs);
  return <div className="timeline-scroll">{groupFailure}
    {!lanes.length ? <WidgetState kind="empty" message={data.unobserved.some(gap => gap.firstHour <= at && at <= gap.lastHour) ? 'Not recorded.' : 'No wars yet.'} />
      : <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}:${viewpointTag ?? ''}`} label="Wars timeline"
        extent={extent} playhead={playhead} liveHead={liveHead} labelHour={at} lanes={lanes} emptyLabel="Not recorded"
        labelCatalogue={prepared.labelCatalogue} preserveZoomOnExtentChange={grow}
        showIcons={false} showNames={false} active={active} unobserved={data.unobserved}
        onHover={hour => setHoverHour(hour === null ? null : Math.floor(hour))}
        onLaneActivate={activate} onLanesInView={onLanesInView}
        revision={`${revision}:${looks.revision}:${hover.revision}:${losses.data?.hour ?? ''}:${contributions.data?.hour ?? ''}`} tooltip={tooltip}
        description={`${summary}. ${options.view === 'gantt' ? 'Enter or Space expands or collapses a war. ' : ''}Arrow keys step through lanes and bars.`}
        onContextMenu={(hour, x, y) => {
          const bounded = Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, hour));
          menu.open(x, y, [jumpItem(runtime, track.id, bounded)]);
        }} />}
    {menu.menu}
  </div>;
}
