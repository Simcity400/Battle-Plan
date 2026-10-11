import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import type { EventBadge } from '../../charts/badges';
import { dayLabel, drawBandLine, drawBands } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { nudgeApart } from '../../charts/palette';
import { stackedSegments, stackExtent } from '../../charts/stack';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { movingRowClass, useRowMotion } from '../../charts/useRowMotion';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupRuns, worldFilter } from '../../dashboard/groups';
import { Lineage } from '../../dashboard/identity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import type { TensionEvent, TensionReply } from '../../generated/dashboard';
import { mix, parseColor, toHex } from '../../shell/appearance/color';
import { OPTIONS } from './definition';
import TensionDrill from './TensionDrillView';
import { pinTensionDrill, TENSION_BREAKDOWN } from './tensionDrill';
import { addedAt, contributionAt, contributionPercent, eventGroup, eventsOutsideReads, eventsWithin, formatContribution, formatTension, formatTensionAxis,
  layerKeys, layerRaw, layerTagAt, otherRaw, rankedAt, readAt, tensionModel, tensionPercent, tensionPlot, validateTensionReply } from './tensionData';
import './worldTension.css';

const NO_WARNINGS: string[] = [];
/** The layer of every country outside the top ones: a key no tag can be, as tags hold no NUL. */
const OTHER = '\u0000other';
/** Events a compact grouped-marker tooltip names before counting the rest. */
const COMPACT_EVENTS = 5;

/** World tension over time: stacked areas of the countries whose
 * actions generated it, each in its recorded map colour, with world tension as a
 * line on top, the events that changed it as triangles along the top, and the
 * values at the moment shown listed beside the chart. A world widget
 * (`countries.all`): chosen countries or a group narrow the layers to them. */
export default function WorldTensionWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const [drill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || (drill.breakdown != null && drill.follow)));
  const [selectedCause, setSelectedCause] = useState<number | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'world-tension', params: { branchId: track.branch.id } } : null;
  const { data, error, retry } = useProduct<TensionReply>(runtime.client, request, active, validateTensionReply);
  // Chosen countries or a group narrow the layers; all countries by default.
  const groups = useCountryGroups(runtime, track, active);
  const lineage = useMemo(() => new Lineage(groups.data?.identityChanges ?? []), [groups.data?.identityChanges]);
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : null, [groups.data]);
  const model = useMemo(() => data ? tensionModel(data, lineage, track?.source.metadata.countries) : null, [data, lineage, track?.source]);
  const dashboardTag = track ? defaultCountryTag(track) : null;

  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  const pinnedDrill = options.show === 'breakdown';
  const followHour = runtime.clock.getHour();
  const drillAt = Math.floor((pinnedDrill && drill.breakdown == null ? head : drillHour(drill, followHour ?? extent[1])) ?? head);
  const resetKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, options.show]);
  useEffect(() => { dispatchDrill({ type: 'close' }); setSelectedCause(null); }, [resetKey]);
  // A group's members as its membership setting counts them at the moment shown.
  const lastHour = track?.branch.lastHour ?? 0;
  const filter = useMemo(() => worldFilter(widget.countries, dashboardTag ? [dashboardTag] : [], groups.data, head, lastHour),
    [widget.countries, dashboardTag, groups.data, head, lastHour]);
  const membership = useMemo(() => {
    if (widget.countries.kind !== 'group' || widget.countries.membership !== 'hour' || !groups.data || !identities) return null;
    const byCountry = new Map<string, { firstHour: number; lastHour: number }[]>();
    for (const span of identities.membershipRuns(groupRuns(groups.data, widget.countries.group) ?? [])) {
      const key = identities.keyAt(span.tag, span.firstHour)!;
      const spans = byCountry.get(key);
      if (spans) spans.push(span); else byCountry.set(key, [span]);
    }
    return byCountry;
  }, [widget.countries, groups.data, identities]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const share = options.values !== 'recorded';
  const count = Number(options.layers) || 5;

  // The layers are the largest contributors at the moment shown, each a country
  // identity (`countryKey`: two countries sharing a tag are two layers), named and
  // coloured by its tag; the stack is rebuilt only when they change.
  const keys = useMemo(() => model ? layerKeys(model, head, count, filter.tags, filter.countryIds) : [], [model, head, count, filter.tags, filter.countryIds]);
  const keysKey = JSON.stringify(keys);
  const tags = useMemo(() => model ? keys.map(key => layerTagAt(model, key, head)) : [], [model, keys, head]);
  const stack = useMemo(() => model ? tensionPlot(model, JSON.parse(keysKey) as string[], share, membership, true, filter.countryIds) : null,
    [model, keysKey, share, membership, filter.countryIds]);
  const events = useMemo(() => model && options.markers !== false ? eventsWithin(model.reply.events, extent[0], extent[1]) : [], [model, options.markers, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps

  // Colours, and the list's names and flags, as recorded near the moment shown (about
  // once a month of playback, at a recorded hour of the moment's own stretch,
  // `useCountryLooks`): each part is held only while its next read loads, never across
  // a gap, and one the recording does not give is not known.
  const lookTags = useMemo(() => [...new Set([...tags.filter((tag): tag is string => tag !== null), ...events.flatMap(event => event.tag == null ? [] : [event.tag])])], [tags, events]);
  const identityHistories = useMemo(() => new Map(groups.data?.countryIdentities.map(history => [history.tag, history.identities]) ?? []), [groups.data]);
  const segmentAt = useCallback((tag: string, hour: number) => runAt(identityHistories.get(tag) ?? [], hour)?.firstHour ?? null, [identityHistories]);
  const looks = useCountryLooks(runtime.client, track, lookTags, head, active, segmentAt);
  const colourOf = useMemo(() => {
    // Nudged apart in a fixed order (by tag) when the app asks for fixed colours, so a
    // country keeps its colour as ranks change; one with no recorded colour is grey.
    const ordered = [...lookTags].sort();
    const known = ordered.filter(tag => looks.color(tag) != null);
    const raw = known.map(tag => { const [r, g, b] = looks.color(tag)!; return { r, g, b }; });
    const adjusted = tokens.countryColours === 'fixed' ? nudgeApart(raw, parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : raw;
    const colours = new Map(known.map((tag, index) => [tag, toHex(adjusted[index])]));
    return (tag: string | null | undefined) => (tag != null ? colours.get(tag) : undefined) ?? tokens.muted;
  }, [lookTags, looks, tokens]);
  const otherColour = mix(tokens.surface, tokens.text, 0.3);
  const layerColours = useMemo(() => [...tags.map(tag => colourOf(tag)), otherColour], [tags, colourOf, otherColour]);

  // A tooltip names its countries as recorded at the hour hovered, asking only for those.
  const metadataNames = useMemo(() => new Map(track?.source.metadata.countries.map(country => [country.tag, country.name])), [track?.source]);
  const metadataName = (tag: string) => metadataNames.get(tag) ?? 'Country name unavailable';
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const hover = useHoverLabels(runtime.client, track, active, useMemo(() => (tag: string) => ({ name: metadataName(tag) }), [metadataNames]), undefined, segmentAt);

  const menu = useContextMenu('Chart actions');
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...(groups.error ? [`Country identity changes could not be loaded: ${groups.error}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, onDiagnostics]);

  const layers = useMemo<ChartLayer[]>(() => !model || !stack ? [] : [{
    draw: (ctx, frame, chartTokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of stackedSegments(stack, frame.x[0], to, hoursPerPixel / 2)) {
        ctx.save();
        ctx.globalAlpha = 0.85;
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands.slice(0, -1), layerColours);
        ctx.restore();
        drawBandLine(ctx, frame, chartTokens, segment.hours, segment.bands.at(-1)!.high, chartTokens.text);
      }
    },
    extent: x => stackExtent(stack, x[0], Math.min(x[1], until ?? x[1])),
  }], [model, stack, layerColours, until]);

  const badges = useMemo(() => new Map<EventBadge, TensionEvent>(events.map(event => [{ hour: event.hour, glyph: '▼', label: event.reason, color: colourOf(event.tag),
    shape: 'triangle' as const }, event])), [events, colourOf]);
  const badgeList = useMemo(() => [...badges.keys()], [badges]);
  // The keyboard steps through every read and every event marked.
  const keyHours = useMemo(() => model ? [...new Set([...model.reply.reads.map(read => read.firstHour), ...events.map(event => event.hour)])].sort((a, b) => a - b) : [], [model, events]);

  // The values listed beside the chart, at the moment shown.
  const shownRead = model ? readAt(model, head) : null;
  const listItems = useMemo(() => {
    if (!model || !shownRead) return [];
    const rows = keys.map((key, index) => ({ key, item: { tag: tags[index], raw: layerRaw(model, shownRead, contributionAt(model, key, head) ?? '0', share) } }));
    rows.push({ key: OTHER, item: { tag: OTHER, raw: layerRaw(model, shownRead, otherRaw(model, shownRead, keys), share) } });
    return rows;
  }, [model, shownRead, keys, tags, head, share]);
  const listKey = JSON.stringify(listItems.map(row => [row.key, row.item.tag, row.item.raw]));
  const stableItems = useMemo(() => listItems, [listKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const moving = useRowMotion(stableItems);

  if (!track) return <WidgetState kind="empty" icon="alert" message="Load a campaign to see world tension." />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (groups.error && widget.countries.kind !== 'all') return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (!model || (widget.countries.kind !== 'all' && !identities)) return <WidgetState kind="loading" label="Loading world tension…" />;
  if (filter.reason) return <WidgetState kind="unavailable" message={filter.reason} />;
  if (!model.reply.reads.length) return <WidgetState kind="unavailable" message="No world tension was recorded in this branch." />;

  const otherCount = (hour: number) => Math.max(0, rankedAt(model, hour).length - keys.length);
  // The moment the chart shows: the playhead, else the end of the hours shown.
  const momentShown = Math.floor(until ?? extent[1]);
  // An event: its country (named as at the hour hovered, the event's own), its date, the
  // tension it generated as first read, and, when the game changed it since, what it
  // adds by the moment shown.
  const eventRows = (list: readonly TensionEvent[], hour: number, divider: boolean): TooltipRow[] => list.flatMap((event, index) => {
    const label = event.tag == null ? { name: 'A country the recording does not name', flag: undefined } : hover.labelAt(hour)(event.tag);
    const first = event.added[0].addedRaw;
    const now = addedAt(event, momentShown);
    return [
      { label: event.reason, strong: true, divider: divider || index > 0 },
      { label: label.name, countryFlag: event.tag == null ? undefined : label, swatch: colourOf(event.tag), value: dayLabel(event.hour) },
      { label: 'Tension generated', value: formatContribution(model, first, true) },
      ...(now !== first ? [{ label: `Adds by ${dayLabel(momentShown)}`, value: formatContribution(model, now, true), tone: 'muted' as const }] : []),
    ];
  });
  const tooltip = (hour: number, hovered: readonly EventBadge[]): TooltipContent | null => {
    // The badges hovered, else (a keyboard stop, which hovers no badge) the events
    // marked at an hour no read holds, so those read as their markers do.
    const fromBadges = hovered.map(badge => badges.get(badge)).filter((event): event is TensionEvent => event != null);
    const marked = fromBadges.length ? fromBadges : eventsOutsideReads(model, events, hour);
    if (marked.length) {
      hover.ask(hour, marked.flatMap(event => event.tag == null ? [] : [event.tag]));
      if (marked.length === 1) return fitTooltip([{ title: marked[0].reason, rows: eventRows(marked, hour, false).slice(1) }], viewport);
      // Several events at one hour: every one in full, else a compact form (their
      // total, then the largest, one row each) cut to the window; the full list is read out.
      const full: TooltipContent = { title: `${marked.length} events on ${dayLabel(hour)}`, rows: eventRows(marked, hour, false) };
      const group = eventGroup(marked);
      const compact: TooltipContent = {
        title: `${marked.length} events · ${formatContribution(model, group.totalRaw, true)} tension`,
        subtitle: dayLabel(hour),
        rows: [
          ...group.largest.slice(0, COMPACT_EVENTS).map((event): TooltipRow => {
            const label = event.tag == null ? { name: 'Not named', flag: undefined } : hover.labelAt(hour)(event.tag);
            return { label: `${label.name}: ${event.reason}`, countryFlag: event.tag == null ? undefined : label, swatch: colourOf(event.tag), value: formatContribution(model, event.added[0].addedRaw, true), short: true };
          }),
          ...(marked.length > COMPACT_EVENTS ? [{ label: `${marked.length - COMPACT_EVENTS} more events`, tone: 'muted' as const }] : []),
        ],
      };
      return fitTooltip([full, compact], viewport, { full, more: hidden => `${hidden} more ${hidden === 1 ? 'row' : 'rows'}: a taller window shows them` });
    }
    const read = readAt(model, hour);
    if (!read) return { title: dayLabel(hour), rows: [{ label: 'World tension', value: 'Not recorded', tone: 'muted' }] };
    const hoveredFilter = membership ? worldFilter(widget.countries, dashboardTag ? [dashboardTag] : [], groups.data, hour, lastHour) : null;
    const ranked = rankedAt(model, hour, hoveredFilter?.tags ?? null, hoveredFilter?.countryIds);
    const here = events.filter(event => event.hour === hour);
    hover.ask(hour, [...ranked.flatMap(item => item.tag === null ? [] : [item.tag]), ...here.flatMap(event => event.tag == null ? [] : [event.tag])]);
    const named = hover.labelAt(hour);
    const rows: TooltipRow[] = [{ label: 'World tension', value: formatTension(model, read.tensionRaw), strong: true }];
    if (contributionPercent(model, read.sumRaw) !== tensionPercent(model, read.tensionRaw)) {
      rows.push({ label: 'Its sources add up to', value: formatContribution(model, read.sumRaw), tone: 'muted' });
    }
    ranked.forEach((item, index) => {
      const label = item.tag === null ? { name: 'Native country identity not recorded', flagReason: 'Native country identity not recorded at this hour.' } : named(item.tag);
      rows.push({ label: label.name, countryFlag: label, swatch: keys.includes(item.key) ? colourOf(item.tag) : undefined,
        value: formatContribution(model, item.raw, true), short: true, divider: index === 0 });
    });
    if (read.unnamedRaw !== '0') rows.push({ label: 'Countries the recording does not name', value: formatContribution(model, read.unnamedRaw, true), short: true });
    rows.push(...eventRows(here, hour, true));
    const notes = [
      ...(hoveredFilter?.reason ? [hoveredFilter.reason] : []),
    ];
    return fitTooltip([{ title: dayLabel(hour), rows, note: notes.join(' ') || undefined }], viewport, { more: hidden => `${hidden} more ${hidden === 1 ? 'row' : 'rows'}: a taller window shows them` });
  };

  const open = (hour: number) => {
    dispatchDrill({ type: 'open', breakdown: TENSION_BREAKDOWN.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: followHour });
    const causes = model.reply.events.flatMap((event, index) => event.hour === hour ? [index] : []);
    setSelectedCause(causes.length === 1 ? causes[0] : null);
  };
  const breakdown = <TensionDrill model={model} runtime={runtime} track={track} active={active} options={options}
    extent={extent} playhead={drill.breakdown != null && drill.follow ? followHour : playhead} hour={drillAt} selectedCause={selectedCause} onCause={setSelectedCause}
    onPick={hour => dispatchDrill(pinnedDrill ? { type: 'open', breakdown: TENSION_BREAKDOWN.id, hour, playing: false, playhead: null }
      : { type: 'pick', hour })} />;
  if (pinnedDrill) return <div className="world-tension-pinned">
    <p className="drill-empty">{formatDateTime(drillAt)} {followHour != null && (drill.breakdown == null ? widget.time.mode !== 'follow' : !drill.follow)
      && <button type="button" className="button small" onClick={() => {
        const hour = runtime.clock.getHour();
        if (hour != null) dispatchDrill({ type: 'open', breakdown: TENSION_BREAKDOWN.id, hour, playing: true, playhead: hour });
      }}>Follow playback</button>}</p>{breakdown}</div>;
  const total = shownRead ? formatTension(model, shownRead.tensionRaw) : 'Not recorded';
  const layerName = (tag: string | null) => tag === OTHER ? 'Other' : tag === null ? 'Native country identity not recorded' : looks.name(tag);
  const overviewState = widget.countries.kind === 'group' && groups.error
    ? <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />
    : filter.reason ? <WidgetState kind="empty" message={filter.reason} />
      : widget.countries.kind === 'group' && !groups.data ? <WidgetState kind="loading" label="Loading country group…" /> : null;
  const main = overviewState ?? <div className={`world-tension${options.list !== false ? ' has-list' : ''}`}>
    <p className="sr-only" aria-live="off">{`${dayLabel(head)}: world tension ${total}`}{shownRead ? `; ${listItems.map(row => `${layerName(row.item.tag)} ${formatContribution(model, row.item.raw)}`).join(', ')}` : ''}</p>
    <div className="world-tension-chart">
      <TimeChart label="World tension and the countries that generated it" extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        // 0–100%, grown to whatever is drawn beyond it (negative parts, or sources past the cap), never cut.
        fitValues={shown => [Math.min(0, shown?.[0] ?? 0), Math.max(100, shown?.[1] ?? 100)]}
        layers={layers} active={active && (drill.breakdown == null || options.drill === 'popout')}
        revision={`${head}:${model.reply.branchId}:${keysKey}:${share}:${looks.revision}:${hover.revision}:${tokens.countryColours}:${viewport}`}
        formatValue={formatTensionAxis}
        badges={badgeList.length ? badgeList : undefined} keyHours={keyHours}
        tooltip={(hour, _frame, hovered) => tooltip(hour, hovered)}
        onPick={open}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {options.list !== false && <aside className="world-tension-list" aria-label={`World tension on ${dayLabel(head)}`}>
      <button type="button" className="world-tension-total" onClick={() => open(head)}><span>World tension</span><b>{total}</b></button>
      <ol style={{ height: `calc(${moving.filter(row => row.phase !== 'leaving').length} * var(--world-tension-row))` }}>
        {moving.map(row => {
          const other = row.item.tag === OTHER;
          const others = other ? otherCount(head) : 0;
          const flag = other || row.item.tag === null ? undefined : looks.flag(row.item.tag);
          return <li key={row.key} className={movingRowClass(row)} style={{ transform: `translateY(calc(${row.slot} * var(--world-tension-row)))` }}>
            <i style={{ background: other ? otherColour : colourOf(row.item.tag) }} aria-hidden="true" />
            {flag ? <img src={flag} alt="" /> : null}
            <span>{other ? `Other${others ? ` (${others})` : ''}` : layerName(row.item.tag)}</span>
            <b>{formatContribution(model, row.item.raw)}</b>
          </li>;
        })}
      </ol>
      {share && shownRead && BigInt(shownRead.sumRaw) !== 0n && contributionPercent(model, shownRead.sumRaw) !== tensionPercent(model, shownRead.tensionRaw)
        && <p className="world-tension-note">Shares of world tension. Its sources add up to {formatContribution(model, shownRead.sumRaw)}.</p>}
    </aside>}
    {menu.menu}
  </div>;
  return <div ref={anchor} style={{ height: '100%' }}><DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'}
    open={drill.breakdown != null} anchor={anchor} title={TENSION_BREAKDOWN.label} day={formatDateTime(drillAt)} following={drill.follow}
    canFollow={followHour != null} onClose={() => dispatchDrill({ type: 'close' })} onFollow={() => dispatchDrill({ type: 'follow' })}
    onPin={() => runtime.addWidgetBelow(widget.id, pinTensionDrill(widget, track, drillAt, drill.follow))} main={main}>{breakdown}</DrillHost></div>;
}
