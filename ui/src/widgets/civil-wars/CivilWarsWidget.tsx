import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { laneRequests } from '../../charts/timeline/laneRequests';
import Timeline from '../../charts/timeline/Timeline';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { flagReadMetadata } from '../../dashboard/countryFlagsAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import type { NameAt } from '../../dashboard/countryFacts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { nameKey, useCountryFactsByKey } from '../../dashboard/countryNamesAt';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useHeadHour } from '../army-size/headHour';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { changesBy } from '../../charts/timeline/runState';
import { tagAt, unknownHours } from '../../dashboard/politicsHistory';
import { useProduct } from '../../dashboard/products';
import { countryReadName, STYLES, useCountriesAt } from '../../dashboard/useCountriesAt';
import { useCountryFlagsRead } from '../../dashboard/countryFlags';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { mix } from '../../shell/appearance/color';
import { sideColours } from './sideColours';
import { OPTIONS } from './definition';
import { changeHours, civilWarLanes, civilWarsAt, jumpHours, laneCountries, NAME_LOADING, NAME_NOT_KNOWN, sideNameHour, sideTooltip, summaryText,
  tooltipCountries, unknownLook, validateCivilWarsReply, warTooltip } from './civilWarsData.ts';
import type { CivilWarLaneInfo, Look } from './civilWarsData';
import './civilWars.css';
import { motionOf } from '../../charts/easing.ts';
import type { CivilWarsReply } from '../../generated/dashboard';
import { recordedHour } from '../../dashboard/regionTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const WORDS = { pending: NAME_LOADING, missing: NAME_NOT_KNOWN };
const pendingCountry = () => ({ name: NAME_LOADING });

export default function CivilWarsWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const followsClock = widget.time.mode === 'follow';
  const structure = useRef<(hour: number) => string>(() => '');
  const playbackHour = useHeadHour(clock, active && followsClock, hour => structure.current(hour));
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'civil-wars', params: { branchId: track.branch.id } }) : null;
  const product = useProduct<CivilWarsReply>(runtime.client, request, active, validateCivilWarsReply, track?.source);
  const data = product.data;
  const branchFirst = track?.branch.firstHour ?? 0;
  const unread = useMemo(() => unknownHours(track?.branch.gaps ?? [], data?.known ?? [], branchFirst, data?.lastHour ?? null),
    [track?.branch.gaps, data, branchFirst]);
  const groups = useCountryGroups(runtime, track, active);
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : null, [groups.data]);
  const axisOffset = track?.axisOffset ?? 0;
  const changes = useMemo(() => data ? changeHours(data, unread, groups.data?.identityChanges) : [], [data, unread, groups.data]);
  structure.current = hour => `${changesBy(changes, Math.floor(hour + axisOffset))}:${clock.getSnapshot().playing ? '' : hour}`;
  const liveHead = useMemo(() => followsClock && track ? { read: () => clock.getSnapshot().position + axisOffset,
    subscribe: (changed: () => void) => clock.subscribeFrame(changed) } : undefined, [followsClock, track, clock, axisOffset]);
  useShownEachFrame(clock, () => data && track ? liveHead ? recordedHour(track, liveHead.read()) : at : null);
  const dashboardTag = track ? defaultCountryKey(track) : null;
  const lastHour = track?.branch.lastHour ?? 0;
  const filter = useMemo(() => worldFilter(widget.countries, dashboardTag ? [dashboardTag] : [], groups.data, at, identities),
    [widget.countries, dashboardTag, groups.data, at, lastHour, identities]);
  const narrowing = widget.countries.kind !== 'all';
  const view = useMemo(() => data ? civilWarsAt(data, at, {
    ended: options.ended === 'shown', countryIds: narrowing ? filter.countryIds ?? new Set() : null, identities, unread, lastHour: data.lastHour,
    ...(widget.time.mode === 'range' ? { from: first } : {}),
  }) : null, [data, at, options.ended, narrowing, filter.countryIds, identities, unread, widget.time.mode, first]);

  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => { setCollapsed(new Set()); }, [track?.source.id, track?.branch.id]);
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const [wanted, setWanted] = useState<{ live: string[]; dated: NameAt[] }>({ live: [], dated: [] });
  const [nameWanted, setNameWanted] = useState<{ live: string[]; dated: NameAt[] }>({ live: [], dated: [] });
  const [tipWanted, setTipWanted] = useState<NameAt[]>([]);
  const tipKey = useRef('');
  const askTip = useCallback((pairs: NameAt[]) => {
    const key = JSON.stringify(pairs);
    if (key === tipKey.current) return;
    tipKey.current = key;
    queueMicrotask(() => setTipWanted(pairs));
  }, []);
  const identityHistories = useMemo(() => new Map(groups.data?.countryIdentities.map(history => [history.tag, history.identities]) ?? []), [groups.data]);
  const segmentAt = useCallback((tag: string, hour: number) => runAt(identityHistories.get(tag) ?? [], hour)?.firstHour ?? null, [identityHistories]);
  const { country, revision } = useCountriesAt(runtime.client, track, wanted.live, at, active, nameWanted.live, segmentAt, WORDS,
    { clock, on: followsClock, hour: (clockHour: number) => track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null });
  const looks = useCountryLooks(runtime.client, track, wanted.live, at, active, segmentAt);
  const datedWanted = useMemo(() => {
    const pairs = new Map<string, NameAt>();
    for (const pair of [...nameWanted.dated, ...tipWanted]) pairs.set(nameKey(pair.hour, pair.tag), pair);
    return [...pairs.values()];
  }, [nameWanted.dated, tipWanted]);
  const datedHours = Math.max(MAX_NAME_HOURS, new Set(datedWanted.map(pair => pair.hour)).size);
  const styles = useCountryFactsByKey(runtime.client, track, datedWanted, active, STYLES, datedHours);
  const flagWanted = useMemo(() => [...wanted.dated, ...tipWanted], [wanted.dated, tipWanted]);
  const flags = useCountryFlagsRead(runtime.client, track, flagWanted, active);
  const fixed = tokens.countryColours === 'fixed';
  const colours = useMemo(() => {
    const keys = [...wanted.live, ...wanted.dated.map(pair => nameKey(pair.hour, pair.tag))];
    const recorded = new Map<string, ReturnType<typeof looks.color>>([
      ...wanted.live.map(tag => [tag, looks.color(tag, at)] as const),
      ...wanted.dated.map(pair => {
        const style = styles.get(nameKey(pair.hour, pair.tag));
        return [nameKey(pair.hour, pair.tag), style && !(style instanceof Error) ? style.color : null] as const;
      }),
    ]);
    return sideColours(keys, key => recorded.get(key) ?? null, tokens.surface, fixed);
  }, [wanted, looks, at, styles, tokens.surface, fixed]);
  const dated = useCallback((tag: string, hour: number): Look => {
    const key = nameKey(hour, tag);
    const style = styles.get(key);
    return { ...countryReadName(style, { name: NAME_LOADING }, NAME_NOT_KNOWN), ...flagReadMetadata(flags.get(key)), color: colours.get(key) };
  }, [styles, flags, colours]);
  const side = useCallback((tag: string, hour: number, live: boolean): Look => live ? { ...country(tag), color: colours.get(tag) } : dated(tag, hour),
    [country, colours, dated]);
  const neutral = mix(tokens.text, tokens.surface, 0.45);
  const { lanes, info } = useMemo(() => view ? civilWarLanes(view.wars, collapsed, { side, dated, day: dayLabel, neutral, positive: tokens.positive }, branchFirst, unread)
    : { lanes: [], info: new Map<string, CivilWarLaneInfo>() }, [view, collapsed, side, dated, neutral, tokens.positive, branchFirst, unread]);
  const requests = laneRequests(lanes, inView);
  const wantedKey = JSON.stringify(laneCountries(requests.looks, info)), nameWantedKey = JSON.stringify(laneCountries(requests.names, info));
  useEffect(() => { setWanted(JSON.parse(wantedKey) as { live: string[]; dated: NameAt[] }); }, [wantedKey]);
  useEffect(() => { setNameWanted(JSON.parse(nameWantedKey) as { live: string[]; dated: NameAt[] }); }, [nameWantedKey]);
  const onLanesInView = useCallback((start: number, end: number) => setInView([start, end]), []);
  const activate = useCallback((lane: TimelineLane) => {
    const about = info.get(lane.id);
    if (about?.kind !== 'war') return;
    setCollapsed(previous => {
      const next = new Set(previous);
      if (next.has(about.war.war.id)) next.delete(about.war.war.id); else next.add(about.war.war.id);
      return next;
    });
  }, [info]);
  const hover = useHoverLabels(runtime.client, track, active, pendingCountry, NAME_NOT_KNOWN, segmentAt);
  const menu = useContextMenu('Timeline actions');

  const unnamedDrawn = requests.looks.filter(lane => {
    const about = info.get(lane.id);
    return about?.kind === 'side' && about.side.tag !== null && lane.identity?.label === NAME_NOT_KNOWN;
  }).length;
  useEffect(() => {
    const unnamed = (view?.unnamed ?? 0) + unnamedDrawn;
    const warnings = [...new Set([...(data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...(looks.warnings ?? []),
      ...(groups.error ? [`Country history unavailable: ${groups.error.message}`] : []),
      ...(unnamed ? [`${unnamed} civil-war ${unnamed === 1 ? 'name is' : 'names are'} not known; shown as “${NAME_NOT_KNOWN}”.`] : [])])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, looks.warnings, view, unnamedDrawn, onDiagnostics]);

  const header = headerSlot && view && createPortal(<span className="pill civil-wars-header-count">{summaryText(view)}</span>, headerSlot);
  const body = (() => {
    if (!track) return <WidgetState kind="empty" icon="flag-torn" message="Load a campaign to see its civil wars." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="outside" />;
    if (product.error) return <WidgetState kind="error" failure={product.error} onRetry={product.retry} />;
    if (narrowing && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (!data || !view || narrowing && !identities) return <WidgetState kind="loading" label="Loading civil wars…" />;
    if (narrowing && filter.reason) return <WidgetState kind="unavailable" message={filter.reason} />;
    if (!lanes.length) {
      const message = view.reached && narrowing && !view.total ? 'No civil wars for these countries.'
        : view.total ? 'No ongoing civil wars.'
          : playhead != null ? 'No civil wars yet.' : widget.time.mode === 'range' ? 'No civil wars in this range.' : 'No civil wars in this campaign.';
      return <WidgetState kind="empty" icon="flag-torn" message={message} />;
    }
    const text = { dated, day: dayLabel, branchFirst, unread };
    const tooltip = (lane: TimelineLane, _span: TimelineSpan | null, hovered: number, full = false): TooltipContent | null => {
      const about = info.get(lane.id);
      if (!about) return null;
      askTip(tooltipCountries(about));
      if (about.kind === 'war') {
        const content = warTooltip(about.war, hovered, text);
        return full ? content : fitTooltip([content], viewport, { more: hidden => `${hidden} more ${hidden === 1 ? 'side' : 'sides'}` });
      }
      const nameHour = sideNameHour(about.side, hovered);
      const tag = tagAt(about.side.side.tags, nameHour);
      if (tag !== null) hover.ask(nameHour, [tag]);
      const look = tag === null ? unknownLook() : hover.labelAt(nameHour)(tag);
      return sideTooltip(about.war, about.side, hovered, look, tag, text);
    };
    const said: { war: string; sides: string[] }[] = [];
    for (const lane of lanes) {
      if (lane.heading) said.push({ war: `${lane.identity?.label}, ${lane.label}`, sides: [] });
      else said.at(-1)?.sides.push(lane.pill ? `${lane.identity?.label}, ${lane.pill.text}` : `${lane.identity?.label}, ${lane.label}`);
    }
    return <>
      <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Civil wars" extent={extent} playhead={playhead} liveHead={liveHead}
        labelHour={liveHead ? undefined : at}
        preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={false} active={active}
        unobserved={unread}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
        onLaneActivate={activate} onLanesInView={onLanesInView}
        revision={`${revision}:${looks.revision}:${styles.size}:${flags.size}:${hover.revision}`} tooltip={tooltip}
        description={`${dayLabel(at)}: ${said.map(item => item.sides.length ? `${item.war}: ${item.sides.join('; ')}` : item.war).join('. ')}. Enter or Space expands or collapses a civil war. Arrow keys step through lanes and bars.`}
        onContextMenu={(hour, x, y, lane) => {
          const about = lane ? info.get(lane.id) : undefined;
          const bounded = Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, hour));
          menu.open(x, y, [jumpItem(runtime, track.id, bounded), ...(about ? jumpHours(about).map(item =>
            ({ ...jumpItem(runtime, track.id, item.hour), label: `Jump to ${item.label} · ${dayLabel(item.hour)}` })) : [])]);
        }} />
    </>;
  })();
  return <div className="civil-wars-widget">
    {header}
    {narrowing && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {body}
    {menu.menu}
  </div>;
}
