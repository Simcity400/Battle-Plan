import type { GatedTime } from '../../dashboard/regionTime';
import type { PinnedSpec } from '../../charts/drill/drillState';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { coverageOf } from '../../dashboard/widgetTime';
import {  pinnedBreakdown } from '../../charts/drill/drillState';
import type { EventBadge } from '../../charts/badges';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { nudgeApart } from '../../charts/palette';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useAtHead } from '../../dashboard/atHead.ts';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, resolveCountries } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { usePushedLooksHour } from '../../dashboard/pushedLooks.ts';
import { resolveWindow } from '../../dashboard/model';
import { trackHourOf } from '../../dashboard/settledHour';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf';
import type { ThreatEvent, ThreatEventsReply } from '../../generated/dashboard';
import { mix, parseColor, toHex } from '../../shell/appearance/color';
import { OPTIONS, TENSION_BREAKDOWN } from './definition';
import TensionBreakdown from './TensionBreakdown';
import TensionChart from './TensionChart';
import { addedAt, contributionCell, contributorName, eventBadges, eventGroup, eventsThrough, layerContributions, layerUnits, otherUnits, percent, percentOf,
  validateThreatEvents, validateWorldTension, worldCells } from './tensionData.ts';
import './worldTension.css';
import { at as pickAt, recordedHour, SPEC } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const PARAMS = {};
const CONTRIBUTIONS = { kind: 'threat-contributions', measure: 'contribution' };
const COMPACT_EVENTS = 5;
const formatContribution = (raw: string, decimals: number) => `${formatRaw(raw, decimals, 2)}%`;
const formatPercent = (value: number) => `${Math.round(value * 10) / 10}%`;

export default function WorldTensionWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const onWarnings = useCallback((warnings: readonly string[]) => props.onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [props.onDiagnostics]);
  if (options.view === 'race' || options.view === 'lines') return <MeasureRace widget={props.widget} runtime={props.runtime} active={props.active}
    measure={CONTRIBUTIONS} decimals={5} view={options.view} options={options} label="Contribution to world tension" icon="alert"
    format={formatContribution} formatValue={formatPercent} onWarnings={onWarnings} />;
  if (options.view === 'events') return <ThreatEventsView {...props} />;
  return <TensionView {...props} />;
}

function colourFunction(tags: readonly string[], looks: { color(tag: string): readonly [number, number, number] | null | undefined }, tokens: { countryColours: string; surface: string; muted: string }) {
  const known = tags.filter(tag => looks.color(tag) != null);
  const raw = known.map(tag => { const [r, g, b] = looks.color(tag)!; return { r, g, b }; });
  const adjusted = tokens.countryColours === 'fixed' ? nudgeApart(raw, parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : raw;
  const colours = new Map(known.map((tag, index) => [tag, toHex(adjusted[index])]));
  return (tag: string | null) => (tag != null ? colours.get(tag) : undefined) ?? tokens.muted;
}

export function pinTensionDrill(source: WidgetSpec, trackId: string, time: GatedTime): PinnedSpec {
  return pinnedBreakdown(source, TENSION_BREAKDOWN, trackId, time);
}

function TensionView({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const shown = useShownHour();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const pinnedDrill = options.show === 'breakdown';
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const share = options.values !== 'recorded';
  const count = Number(options.layers) || 5;
  const drillControl = useDrill(runtime, useRegion(widget), pinnedDrill ? TENSION_BREAKDOWN.id : null);
  const drill = drillControl.state;
  const resetKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, options.show]);
  useEffect(() => { drillControl.close(); }, [resetKey]);
  const drawn = useRef<{ count: number; keep: (tag: string) => boolean }>({ count, keep: () => true });
  const gate = useGate(runtime, widget);
  const head = useHeadProduct(runtime, gate, 'world-tension', track ? PARAMS : null, {
    active, validate: validateWorldTension,
    structure: reply => JSON.stringify([layerContributions(reply.contributions, drawn.current.count, drawn.current.keep),
      reply.contributions.length, reply.unnamed, reply.missing, reply.warnings]),
  });
  const drillFollows = drillControl.following;
  const drillCoverage = coverageOf(drillControl.outside);
  const drillAt = drillControl.hour;
  const picked = useHeadProduct(runtime, drillControl.gate, 'world-tension', track && drillControl.open ? PARAMS : null, {
    active: active && drillControl.open, validate: validateWorldTension,
    structure: reply => JSON.stringify([reply.contributions, reply.unnamed, reply.missing, reply.warnings]),
  });
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const hovered = useHeadProduct(runtime, useGate(runtime, widget, hoverHour === null ? SPEC : pickAt(hoverHour)), 'world-tension', track && hoverHour !== null ? PARAMS : null, {
    active: active && hoverHour !== null, validate: validateWorldTension,
  });
  const listHour = head.at ?? Math.floor(playhead ?? extent[1]);

  const narrowed = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const resolved = useMemo(() => narrowed ? resolveCountries(widget.countries, [], groups.data, lastHour, Number.MAX_SAFE_INTEGER) : null,
    [narrowed, widget.countries, groups.data, lastHour]);
  const keepAt = useCallback((hour: number | null) => {
    if (!resolved || resolved.top != null) return () => true;
    const tags = new Set(resolved.tags);
    return (tag: string) => tags.has(tag) && (!resolved.membership || hour !== null && isMember(resolved.membership.get(tag), hour));
  }, [resolved]);
  const keep = useMemo(() => keepAt(listHour), [keepAt, listHour]);
  const top = resolved?.top ?? count;
  const breakdownShown = drillControl.open;
  drawn.current = { count: Math.min(top, count), keep };
  const drillContribution = picked.read === 'followed' ? picked.current() : picked.data;
  const layers = useMemo(() => head.data ? layerContributions(head.data.contributions, Math.min(top, count), keep) : [], [head.data, top, count, keep]);
  const layersKey = JSON.stringify(layers);
  const stableLayers = useMemo(() => layers, [layersKey]);

  const events = useProduct<ThreatEventsReply>(runtime.client, track && options.markers !== false ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'threat-events', params: { branchId: track.branch.id } }) : null, active, validateThreatEvents, track?.source);
  const lookTagsNow = useMemo(() => [...new Set(stableLayers.flatMap(layer => layer.tag ? [layer.tag] : []))].sort(), [stableLayers]);
  const drillLayers = useMemo(() => drillContribution ? layerContributions(drillContribution.contributions, Math.min(top, count), keepAt(drillAt)) : [],
    [drillContribution, top, count, keepAt, drillAt]);
  const drillTagsNow = useMemo(() => [...new Set((drillContribution?.contributions ?? []).flatMap(item => item.tag ? [item.tag] : []))].sort(), [drillContribution]);
  const drillTags = useMemo(() => drillTagsNow, [drillTagsNow.join(',')]);
  const drillLooks = useCountryLooks(runtime.client, track, drillTags, drillAt ?? 0, active && breakdownShown && drillAt !== null);
  const lookTags = useMemo(() => lookTagsNow, [lookTagsNow.join(',')]);
  const named = usePushedLooksHour(runtime, widget, track, lookTags, active && head.read === 'followed', listHour);
  const looks = useCountryLooks(runtime.client, track, lookTags, named.hour, active, undefined, undefined, named.exact);
  useWhyCommit('TensionView', { reply: head.data, at: head.at, read: head.read, picked: picked.data, hovered: hovered.data, hoverHour, drill, layers: stableLayers,
    events: events.data, named: named.hour, looks });
  const colourOf = useMemo(() => colourFunction(lookTags, looks, tokens), [lookTags, looks, tokens]);
  const drillColourOf = useMemo(() => colourFunction(drillTags, drillLooks, tokens), [drillTags, drillLooks, tokens]);
  const otherColour = mix(tokens.surface, tokens.text, 0.3);
  const badges = useMemo(() => eventBadges(events.data?.events ?? [], extent[0], extent[1], colourOf), [events.data, extent[0], extent[1], colourOf]);
  const badgeList = useMemo(() => [...badges.keys()], [badges]);
  const labels = useCountryLabelsAt(runtime.client, track, useMemo(() => (hovered.data?.contributions ?? []).slice(0, 12)
    .flatMap(item => item.tag && hoverHour !== null ? [{ tag: item.tag, hour: hoverHour }] : []), [hovered.data, hoverHour]), active);

  useEffect(() => {
    const warnings = [...(head.data?.warnings ?? []), ...(events.error ? [`Threat events could not be read: ${events.error.message}`] : []),
      ...(groups.error ? [`Country groups could not be loaded: ${groups.error.message}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [head.data, events.error, groups.error, onDiagnostics]);

  const report = useCallback(() => shown(head.shownAt()), [shown, head.shownAt]);
  const texts = useMemo<LiveTexts>(() => ({
    text: key => {
      const reply = head.data;
      if (!reply) return '';
      report();
      const values = head.live.read();
      const world = worldCells(values, reply.unnamed);
      if (key === 'total') return world.tension === null ? 'Not recorded' : percent(String(Math.trunc(world.tension)), reply.tensionScale);
      const listed = stableLayers.map(layer => layerUnits(values.get(contributionCell(layer.slot)) ?? null, world, share));
      if (key === 'other') { const other = otherUnits(listed, world, share); return other === null ? 'Not known' : percentOf(other, reply.contributionScale); }
      const at = stableLayers.findIndex(layer => String(layer.slot) === key);
      const value = at < 0 ? null : listed[at];
      return value === null || value === undefined ? 'Not known' : percentOf(value, reply.contributionScale);
    },
    subscribe: head.live.subscribe,
  }), [head.data, head.live, stableLayers, share, report]);
  useEffect(() => { if (!head.data) shown(null); }, [head.data, shown]);

  const menu = useContextMenu('Chart actions');
  interface Region { moment: number | null; layers: typeof stableLayers; colourOf: (tag: string | null) => string; looks: { name(tag: string): string } }
  const mainRegion: Region = { moment: Math.floor(playhead ?? extent[1]), layers: stableLayers, colourOf, looks };
  const drillRegion: Region = { moment: drillAt === null ? null : Math.floor(drillAt), layers: drillLayers, colourOf: drillColourOf, looks: drillLooks };
  const eventRows = ({ moment: momentShown, colourOf, looks }: Region, list: readonly ThreatEvent[], divider: boolean, scale: number): TooltipRow[] => list.flatMap((event, index) => {
    const first = event.added[0].addedRaw, now = momentShown === null ? first : addedAt(event, momentShown);
    return [
      { label: event.reason, strong: true, divider: divider || index > 0 },
      { label: event.tag ? contributorName(event.tag, event.remembered, tag => looks.name(tag)) : 'Unknown country', swatch: colourOf(event.tag), value: dayLabel(event.hour) },
      { label: 'Tension generated', value: percent(first, scale, 2) },
      ...(now !== first && momentShown !== null ? [{ label: `Adds by ${dayLabel(momentShown)}`, value: percent(now, scale, 2), tone: 'muted' as const }] : []),
    ];
  });
  const tooltipIn = (region: Region) => (hour: number, hoveredBadges: readonly EventBadge[]): TooltipContent | null => {
    const { colourOf, looks, layers } = region;
    const scale = head.data?.contributionScale ?? 100_000;
    const marked = hoveredBadges.map(badge => badges.get(badge)).filter((event): event is ThreatEvent => event != null);
    if (marked.length === 1) return fitTooltip([{ title: marked[0].reason, rows: eventRows(region, marked, false, scale).slice(1) }], viewport);
    if (marked.length > 1) {
      const full: TooltipContent = { title: `${marked.length} events on ${dayLabel(hour)}`, rows: eventRows(region, marked, false, scale) };
      const group = eventGroup(marked);
      const compact: TooltipContent = { title: `${marked.length} events · ${percent(group.totalRaw, scale, 2)} tension`, subtitle: dayLabel(hour), rows: [
        ...group.largest.slice(0, COMPACT_EVENTS).map((event): TooltipRow => ({ label: `${event.tag ? looks.name(event.tag) : 'Not named'}: ${event.reason}`,
          swatch: colourOf(event.tag), value: percent(event.added[0].addedRaw, scale, 2), short: true })),
        ...(marked.length > COMPACT_EVENTS ? [{ label: `${marked.length - COMPACT_EVENTS} more events`, tone: 'muted' as const }] : []),
      ] };
      return fitTooltip([full, compact], viewport, { full, more: hidden => `${hidden} more` });
    }
    const reply = hovered.data;
    if (recordedHour(track, hour) === null) return { title: dayLabel(hour), rows: [{ label: 'World tension', value: 'Outside the recording', tone: 'muted' }] };
    if (!reply || hovered.hour !== Math.floor(hour)) return { title: dayLabel(hour), rows: [{ label: 'World tension', value: hovered.error ? 'Not read' : 'Loading…', tone: 'muted' }] };
    const values = hovered.live.read();
    const world = worldCells(values, reply.unnamed);
    if (world.tension === null) return { title: dayLabel(hour), rows: [{ label: 'World tension', value: 'Not recorded', detail: reply.missing ?? world.why, tone: 'muted' }] };
    const named = labels.labelAt(Math.floor(hour));
    const rows: TooltipRow[] = [{ label: 'World tension', value: percentOf(world.tension, reply.tensionScale, 2), strong: true }];
    if (world.sum !== null && Math.trunc(world.sum / 100) !== Math.trunc(world.tension)) rows.push({ label: 'Its sources add up to', value: percentOf(world.sum, reply.contributionScale, 2), tone: 'muted' });
    reply.contributions.slice(0, 12).forEach((item, index) => {
      const value = values.get(contributionCell(item.slot));
      const label = item.tag ? named(item.tag) : { name: 'Unknown country', flagReason: 'Unknown country at this hour.' };
      rows.push({ label: contributorName(item.tag, item.remembered, () => label.name), countryFlag: item.tag ? label : undefined, swatch: layers.some(layer => layer.slot === item.slot) ? colourOf(item.tag) : undefined,
        value: value === undefined ? 'Not known' : percentOf(value, reply.contributionScale, 2), short: true, divider: index === 0 });
    });
    if (reply.contributions.length > 12) rows.push({ label: `${reply.contributions.length - 12} more countries`, tone: 'muted' });
    if (reply.unnamed) rows.push({ label: 'Countries never named', value: percentOf(world.unnamed, reply.contributionScale, 2), short: true });
    rows.push(...eventRows(region, (events.data?.events ?? []).filter(event => event.hour === Math.floor(hour)), true, reply.contributionScale));
    return fitTooltip([{ title: dayLabel(hour), rows }], viewport, { more: hidden => `${hidden} more` });
  };
  const tooltip = tooltipIn(mainRegion);
  const open = (hour: number) => drillControl.show(TENSION_BREAKDOWN.id, hour);
  const anchor = useRef<HTMLDivElement>(null);
  const followRoot = head.root;
  const rootRef = useCallback((element: HTMLDivElement | null) => { anchor.current = element; followRoot(element); }, [followRoot]);

  if (!track) return <WidgetState kind="empty" icon="alert" message="Load a campaign to see world tension." />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (resolved?.reason) return <WidgetState kind="empty" icon="alert" message={resolved.reason} />;
  const breakdownHour = drillAt;
  const breakdown = <TensionBreakdown runtime={runtime} gate={drillControl.gate} track={track} active={active} options={options} extent={extent} playhead={breakdownHour}
    following={drillFollows} hour={breakdownHour} read={drillCoverage} reply={drillContribution} live={picked.live} shownAt={picked.shownAt}
    error={picked.error}
    onRetry={picked.retry} looks={drillLooks} colour={drillColourOf} otherColour={otherColour} layers={drillLayers} events={events.data?.events ?? []}
    tooltip={hour => tooltipIn(drillRegion)(hour, [])} onHover={hour => setHoverHour(hour === null ? null : Math.floor(hour))}
    onPick={drillControl.pick} />;
  if (pinnedDrill) return <DrillHost drill={drillControl} runtime={runtime} mode="inline" anchor={anchor} title={TENSION_BREAKDOWN.label} root={picked.root}
    day={breakdownHour === null ? '' : formatDateTime(breakdownHour)} canFollow main={null}>{breakdown}</DrillHost>;
  const main = head.error && !head.data ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
    : !head.data ? <WidgetState kind="loading" label="Loading world tension…" /> : <div className={`world-tension${options.list !== false ? ' has-list' : ''}`}>
    <TensionChart runtime={runtime} gate={gate} track={track} active={active && (drill.breakdown == null || options.drill === 'popout')} options={options}
      label="World tension and the countries that generated it" extent={extent} playhead={playhead} following={following} layers={stableLayers}
      colours={stableLayers.map(layer => colourOf(layer.tag))} otherColour={otherColour} share={share} live={head.read === 'followed' ? head.live : null}
      badges={options.markers !== false ? badgeList : undefined} tooltip={tooltip} onHover={hour => setHoverHour(hour === null ? null : Math.floor(hour))}
      onPick={open} onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])}
      revision={`${layersKey}:${share}:${tokens.countryColours}:${viewport}:${hovered.data ? hovered.hour : ''}:${labels.labelAt}`} />
    {options.list !== false && <aside className="world-tension-list" aria-label={`World tension on ${dayLabel(listHour)}`}>
      <button type="button" className="world-tension-total" onClick={() => open(listHour)}><span>World tension</span><b><LiveText live={texts} at="total" /></b></button>
      <ol>
        {stableLayers.map(layer => {
          const flag = layer.tag ? looks.flag(layer.tag) : undefined;
          return <li key={layer.slot}>
            <i style={{ background: colourOf(layer.tag) }} aria-hidden="true" />
            {flag ? <img src={flag} alt="" /> : null}
            <span>{contributorName(layer.tag, layer.remembered, tag => looks.name(tag))}</span>
            <b><LiveText live={texts} at={String(layer.slot)} /></b>
          </li>;
        })}
        <li key="other"><i style={{ background: otherColour }} aria-hidden="true" />
          <span>{`Other${head.data.contributions.length > stableLayers.length ? ` (${head.data.contributions.length - stableLayers.length})` : ''}`}</span>
          <b><LiveText live={texts} at="other" /></b></li>
      </ol>
    </aside>}
    {menu.menu}
  </div>;
  return <div ref={rootRef} style={{ height: '100%' }}>
    <DrillHost drill={drillControl} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor} title={TENSION_BREAKDOWN.label} root={picked.root}
      day={breakdownHour === null ? '' : formatDateTime(breakdownHour)} canFollow
      pinRefused={drillControl.pinRefused} onPin={!drillControl.time() ? undefined : () => { const time = drillControl.time(); if (time) runtime.addWidgetBelow(widget.id, pinTensionDrill(widget, track.id, time)); }}
      main={main}>{breakdown}</DrillHost>
  </div>;
}

function ThreatEventsView({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const shown = useShownHour();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'threat-events', params: { branchId: track.branch.id } }) : null;
  const reply = useProduct<ThreatEventsReply>(runtime.client, request, active, validateThreatEvents, track?.source);
  const trackHour = (clockHour: number) => trackHourOf(widget, track, runtime.tracks, clockHour);
  useAtHead(runtime.clock, active && widget.time.mode === 'follow', clockHour => {
    const at = trackHour(clockHour);
    if (at === null) return -1;
    let count = 0;
    for (const event of reply.data?.events ?? []) if (event.firstRead <= at) count += 1 + event.added.filter(added => added.hour <= at).length + (event.lastRead < at ? 1 : 0);
    return count;
  });
  const hour = trackHour(runtime.clock.getHour());
  useEffect(() => { shown(reply.data ? hour : null); }, [reply.data, hour, shown]);
  const rows = useMemo(() => reply.data && hour !== null ? eventsThrough(reply.data, hour) : [], [reply.data, hour]);
  const [visible, setVisible] = useState<readonly ThreatEvent[]>([]);
  const labels = useCountryLabelsAt(runtime.client, track, useMemo(() => visible.flatMap(event => event.tag ? [{ tag: event.tag, hour: event.firstRead }] : []), [visible]), active);
  const columns = useMemo<readonly TableColumn<ThreatEvent>[]>(() => hour === null ? [] : [
    { key: 'date', header: 'Created', label: 'Created', value: event => event.hour, render: event => dayLabel(event.hour), width: 'max-content' },
    { key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-country-min), 1fr)',
      value: event => event.tag, render: event => event.tag ? contributorName(event.tag, event.remembered, tag => labels.labelAt(event.firstRead)(tag).name) : 'Unknown country' },
    { key: 'reason', header: 'Event', label: 'Event', value: event => event.reason, width: 'minmax(12rem, 3fr)' },
    { key: 'added', header: 'Adds', label: 'Adds', numeric: true, width: 'max-content', value: event => BigInt(addedAt(event, hour)),
      render: event => `${formatRaw(addedAt(event, hour), 5, 2)}%` },
    { key: 'until', header: 'Listed until', label: 'Listed until', width: 'max-content', value: event => event.lastRead,
      render: event => event.lastRead >= hour ? 'Still listed' : dayLabel(event.lastRead) },
  ], [labels, hour]);
  useEffect(() => {
    const warnings = reply.data?.warnings ?? [];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [reply.data, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="alert" message="Load a campaign to see its threat events." />;
  if (hour === null) return <WidgetState kind="outside" />;
  if (reply.error) return <WidgetState kind="error" failure={reply.error} onRetry={reply.retry} />;
  if (!reply.data) return <WidgetState kind="loading" label="Loading threat events…" />;
  if (!rows.length) return <WidgetState kind="empty" icon="alert" message={`No threat event recorded by ${dayLabel(hour)}.`} />;
  return <Table label="Threat events" rows={rows} columns={columns} rowKey={event => `${event.hour}:${event.firstRead}:${event.reason}:${event.tag ?? ''}`}
    sort={{ column: 'date', direction: 'descending' }} setOption={setOption} onVisibleRows={setVisible} density="compact" />;
}
