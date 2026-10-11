import { useCallback, useEffect, useMemo, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { laneRequests } from '../../charts/timeline/laneRequests';
import Timeline from '../../charts/timeline/Timeline';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { useFactionColours } from '../../dashboard/FactionColours';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { selectedTags } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { STYLES, useCountriesAt } from '../../dashboard/useCountriesAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import type { CountryStyle } from '../../generated/dashboard';
import { mix } from '../../shell/appearance/color';
import { OPTIONS } from './definition';
import { NAME_WORDS, casualtiesAt, countryTagAt, labelObservationAt, laneCountries, observationAt, participantHoldingAt, prepareWars, retainExpandedWars, validateWarsReply, viewpointAt, viewpointSummary, warsLanes, warsReplyState } from './warsData';
import type { CountryResolver, WarsColour, WarsLaneInfo, WarsView } from './warsData';
import { warTooltip } from './warsTooltip';
import { warsColourPlan, warsCountryColours, warsFixedColourKey, warsLookStart, warsSideColours } from './warsColours';

const NO_WARNINGS: string[] = [];
const pendingCountry = () => ({ name: NAME_WORDS.pending });

export default function WarsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 1;
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  // The viewpoint is the widget's country: the dashboard's unless one is picked.
  const viewpointTag = selectedTags(widget.countries, track)[0] ?? null;
  const product = useProduct(runtime.client, track ? { sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpointTag } } : null, active, validateWarsReply);
  const data = product.data;
  const groups = useCountryGroups(runtime, track, active);
  const factions = useFactionColours(runtime.client, track?.source);
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : undefined, [groups.data]);
  const prepared = useMemo(() => data && identities ? prepareWars(data, undefined, identities) : null, [data, identities]);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  // War and branch IDs are small integers local to one recording: they belong to one
  // recording, branch and viewpoint, including its refetches.
  useEffect(() => { setExpanded(new Set()); }, [track?.source.id, track?.branch.id, viewpointTag]);
  useEffect(() => {
    if (prepared) setExpanded(previous => retainExpandedWars(previous, prepared.byId));
  }, [prepared]);
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const [named, setNamed] = useState<string[]>([]);
  const namedSet = useMemo(() => new Set(named), [named]);
  const segmentAt = useCallback((tag: string, hour: number) => warsLookStart(prepared, track?.branch, tag, hour), [prepared, track?.branch]);
  const { country, revision } = useCountriesAt(runtime.client, track, named, at, active, named, segmentAt, NAME_WORDS);
  const looks = useCountryLooks(runtime.client, track, named, at, active, segmentAt);
  const fixed = tokens.countryColours === 'fixed';
  const stylePlan = useMemo(() => warsColourPlan(prepared, track?.branch, at, fixed), [prepared, track?.branch, at, fixed]);
  // Fixed colours need the full stable participant set; artwork still reads visible lanes only.
  const styles = useCountryFactsRead(runtime.client, track, stylePlan.wanted, active, STYLES, stylePlan.maxHours);
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
    palette: tokens.categories, country: countries, named: namedSet, neutral, sideColours,
  }) : { lanes: [], info: new Map<string, WarsLaneInfo>() }, [prepared, at, options.view, options.colour, expanded, tokens.categories, countries, namedSet, neutral, sideColours]);
  const viewpoint = data ? viewpointAt(data, at) : null;
  // Both names and flags ask only for visible lanes, with the shared margin.
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
      ...(factions.error ? [factions.error] : []), ...(groups.error ? [`Native country history could not be read: ${groups.error}`] : []),
      ...(prepared?.unidentifiedHours ? [`Native identity is not recorded for ${prepared.unidentifiedHours} participant-hours; those lanes retain unknown country labels and colours.`] : [])])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, factions.warnings, factions.error, prepared, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its wars." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;
  const state = warsReplyState(data, product.error);
  if (state === 'error') return <WidgetState kind="error" message={product.error!} onRetry={product.retry} />;
  if (state === 'unavailable') return <WidgetState kind="unavailable" />;
  if (groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (!data || !prepared) return <WidgetState kind="loading" label="Loading wars…" />;

  const tooltip = (lane: TimelineLane, span: TimelineSpan | null, hour: number, full = false): TooltipContent | null => {
    const about = info.get(lane.id);
    if (!about) return null;
    const gap = track.branch.gaps.find(gap => gap.firstHour <= hour && hour <= gap.lastHour);
    const recorded = track.branch.firstHour <= hour && hour <= track.branch.lastHour && !gap;
    const datedLabels = recorded ? hover.labelAt(hour) : () => ({ name: NAME_WORDS.missing, known: false });
    const labels = (tag: string) => prepared.identities?.keyAt(tag, hour) === null
      ? { name: NAME_WORDS.missing, known: false } : datedLabels(tag);
    const nameReason = !recorded ? gap?.reason ?? 'This hour is outside the recorded branch.' : undefined;
    const bar = span ? about.spansById.get(span.id) : undefined;
    const war = about.kind === 'country' ? bar?.war : about.war;
    if (!war) {
      if (about.kind !== 'country') return null;
      const tag = countryTagAt(about.country, hour);
      if (recorded && tag !== null) hover.ask(hour, [tag]);
      return { title: tag === null ? NAME_WORDS.missing : labels(tag).name, subtitle: dayLabel(hour), description: nameReason, rows: [{ label: 'War', value: data.unobserved.some(gap => gap.firstHour <= hour && hour <= gap.lastHour)
        ? 'Not recorded' : 'No war observed' }] };
    }
    const observation = observationAt(war, hour);
    const labelled = labelObservationAt(war, hour);
    const tag = bar?.run?.tag ?? (about.kind === 'participant' ? countryTagAt(about.participant, hour)
      : about.kind === 'country' ? countryTagAt(about.country, hour) : null);
    const tags = tag ? [tag, ...(labelled ? [labelled.viewpointTag] : [])]
      : [...new Set([...(labelled ? [labelled.viewpointTag] : []), ...(observation ? [...observation.viewpointSide, ...observation.opposingSide] : [])])];
    if (recorded) hover.ask(hour, tags.filter(tag => prepared.identities?.keyAt(tag, hour) !== null));
    const participant = about.kind === 'participant' ? about.participant : bar?.participant;
    const content = warTooltip({ war, observation, losses: casualtiesAt(war, hour), hour, until: at,
      branchLast: track.branch.lastHour, unobserved: data.unobserved, branchGaps: track.branch.gaps, capitulationsUnobserved: data.capitulationsUnobserved,
      country: labels, day: dayLabel,
      ...(tag ? { participant: { tag, run: bar?.holding ?? (participant ? participantHoldingAt(participant, hour) : null),
        capitulations: participant?.capitulations } } : {}),
    });
    if (nameReason) content.description = nameReason;
    return full ? content : fitTooltip([content], viewport, { more: hidden => `${hidden} more ${hidden === 1 ? 'participant' : 'participants'}` });
  };
  const seenAs = viewpoint ? countries(viewpoint) : null;
  const summary = viewpointSummary(seenAs);
  return <div className="timeline-scroll">
    {!lanes.length ? <WidgetState kind="empty" message={data.unobserved.some(gap => gap.firstHour <= at && at <= gap.lastHour) ? 'Not recorded.' : 'No wars yet.'} />
      : <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}:${viewpointTag ?? ''}`} label="Wars timeline"
        extent={extent} playhead={playhead} labelHour={at} lanes={lanes} emptyLabel="Not recorded"
        labelCatalogue={prepared.labelCatalogue} preserveZoomOnExtentChange={options.grow === true}
        showIcons={false} showNames={false} active={active} unobserved={data.unobserved}
        onLaneActivate={activate} onLanesInView={onLanesInView}
        revision={`${revision}:${looks.revision}:${hover.revision}`} tooltip={tooltip}
        description={`${summary}. ${options.view === 'gantt' ? 'Enter or Space expands or collapses a war. ' : ''}Arrow keys step through lanes and observed bars. Dates marked “by” are observation bounds. Gaps stay blank.`}
        onContextMenu={(hour, x, y) => {
          const bounded = Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, hour));
          menu.open(x, y, [jumpItem(runtime, track.id, bounded)]);
        }} />}
    {menu.menu}
  </div>;
}
