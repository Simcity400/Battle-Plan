import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import type { StabilityPartsReply } from '../../generated/dashboard';
import StabilityDrill, { MEASURE_NAMES } from './StabilityDrill';
import { validateStabilityPartsReply } from './partsData';
import { SourceFlag } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import { dayLabel, drawLine, END_LABEL_HEIGHT, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { categoricalPalette, nudgeApart, SECONDARY_DASH } from '../../charts/palette';
import { extentOf, lineEndHour } from '../../charts/series';
import type { Run } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { CHART_LAYOUT } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { clipRuns, isMember, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { readHourAt, validateStabilityReply } from '../../dashboard/stabilityReply';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CountryStabilityReply, CountryStyle, CountryStyleReply } from '../../generated/dashboard';
import { parseColor, toHex } from '../../shell/appearance/color';
import { STABILITY_BREAKDOWNS, STABILITY_OPTIONS } from './definition';
import { stabilityMarkers } from './markers';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import type { NameAt } from '../../dashboard/countryNamesAt';
import type { MarkerKind } from './markers';
import type { FocusReply, PartyHistoryReply } from '../../generated/dashboard';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply';
import type { IdeaHistoryReply } from '../../generated/dashboard';
import { validatePartyHistoryReply } from '../ruling-party/partyData';
import { validateFocusReply } from '../national-focus/focusData';
import { breakdownRows, formatPercent, linesPerMeasure, percentRuns, rawAt, zoomRange } from './stabilityData';
import type { Measure } from './stabilityData';
import './stability.css';

const NO_WARNINGS: string[] = [];
/** Country names and colours are read at most once per this many hours of playback. */
const STYLE_STEP = 30 * 24;
/** A failed breakdown is asked for again after this delay, doubling each time, this many times. */
const BREAKDOWN_RETRY_MS = 3000;
const BREAKDOWN_RETRIES = 4;
/** With focus in the widget, the breakdown of the moment shown loads once playback rests this long. */
const FOCUS_SETTLE_MS = 400;
/** The gap kept between end labels. */
const END_LABEL_GAP = 2;
const MEASURES: readonly { key: Measure; label: string; dash?: readonly number[] }[] = [
  { key: 'stability', label: 'Stability' },
  { key: 'warSupport', label: 'War support', dash: SECONDARY_DASH },
];

/** Stability and war support over time (mock-ups politics-1/2): each country in its
 * map colour, stability solid and war support dashed, on a fixed 0–100% scale by
 * default. Hovering one country shows the game's own breakdown of both values for
 * the read behind that hour. */
export default function StabilityWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(STABILITY_OPTIONS, widget.options);
  const shown = MEASURES.filter(measure => options[measure.key] !== false);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER), [widget.countries, fallback, groups.data, lastHour]);
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-stability',
    params: { branchId: track.branch.id, tags: resolved.tags, hour: null } } : null;
  const { data, error, retry } = useProduct<CountryStabilityReply>(runtime.client, request, active, validateStabilityReply);

  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const styleAt = playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour);
  // During playback names and colours are read in steps; otherwise at the hour shown.
  const styleHour = !track ? 0 : !following ? styleAt : Math.min(lastHour, track.branch.firstHour + Math.floor((styleAt - track.branch.firstHour) / STYLE_STEP) * STYLE_STEP);
  const styleRequest = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-styles',
    params: { branchId: track.branch.id, hour: styleHour, tags: resolved.tags } } : null;
  const styles = useProduct<CountryStyleReply>(runtime.client, styleRequest, active);
  const [known, setKnown] = useState<ReadonlyMap<string, CountryStyle>>(new Map());
  useEffect(() => setKnown(new Map()), [track?.source.id, track?.source.revision, track?.branch.id]);
  useEffect(() => {
    if (styles.data) setKnown(previous => new Map([...previous, ...styles.data!.styles.map(style => [style.tag, style] as const)]));
  }, [styles.data]);
  // Names and colours kept from another hour are not passed off as this hour's when its read fails.
  useEffect(() => { if (styles.error) setKnown(new Map()); }, [styles.error]);

  // The breakdown of the one country shown, for the read behind the hovered hour,
  // without the histories (breakdownOnly), each read fetched once and cached.
  const single = resolved.tags.length === 1 ? data?.series.find(series => series.tag === resolved.tags[0]) : undefined;
  // Each read is asked for at one shown hour of it (the read's own hour may not be
  // readable), kept per country and read so revisiting reuses the cached reply;
  // an hour that turns out to be a gap is replaced by the next hovered hour.
  const [hover, setHover] = useState<{ tag: string; read: number; hour: number } | null>(null);
  const hoverKey = useRef<string | null>(null);
  const requestHours = useRef(new Map<string, number>());
  const emptyHours = useRef(new Set<string>());
  const retries = useRef(0);
  const tipHour = useRef<number | null>(null);
  const hoverRead = hover && single && hover.tag === single.tag ? hover.read : null;
  const breakdownRequest = track && single && hover && hover.tag === single.tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-stability',
    params: { branchId: track.branch.id, tags: [single.tag], hour: hover.hour, breakdownOnly: true } } : null;
  const breakdown = useProduct<CountryStabilityReply>(runtime.client, breakdownRequest, active, validateStabilityReply);
  // Only the hovered country's breakdown of the hovered read is shown.
  const replied = breakdown.data?.series[0];
  const breakdownFor = replied && single && replied.tag === single.tag && replied.breakdown?.readHour === hoverRead ? replied.breakdown : null;
  // A new recording, branch or country starts afresh.
  useEffect(() => {
    setHover(null); hoverKey.current = null; requestHours.current.clear(); emptyHours.current.clear();
  }, [track?.source.id, track?.source.revision, track?.branch.id, single?.tag]);
  useEffect(() => {
    // An hour with no breakdown, or whose lines were not recorded, is asked again at the next hour hovered.
    if (breakdown.data && hover && (!replied?.breakdown || !replied.breakdown.linesAvailable)) emptyHours.current.add(`${hover.tag}:${hover.read}:${hover.hour}`);
  }, [breakdown.data, hover, replied]);
  // A failed breakdown is asked for again after a growing delay, a few times, even
  // with the pointer still; hovering the read again starts the attempts afresh.
  useEffect(() => { retries.current = 0; }, [hover]);
  // The latest retry, as useProduct makes a new one each render.
  const retryBreakdown = useRef(breakdown.retry);
  retryBreakdown.current = breakdown.retry;
  useEffect(() => {
    if (!breakdown.error || !hover || !active || retries.current >= BREAKDOWN_RETRIES) return;
    const timer = setTimeout(() => {
      retries.current += 1;
      retryBreakdown.current();
    }, BREAKDOWN_RETRY_MS * 2 ** retries.current);
    return () => clearTimeout(timer);
  }, [breakdown.error, hover, active]);


  const series = useMemo(() => {
    if (!data) return [];
    const palette = categoricalPalette(tokens.surface);
    const raw = data.series.map((item, index) => {
      const style = known.get(item.tag);
      // A country without a recorded map colour is drawn in neutral grey, never a colour it lacks.
      const recorded = style?.color ? { r: style.color[0], g: style.color[1], b: style.color[2] } : parseColor(tokens.muted) ?? parseColor(palette[index % palette.length])!;
      const membership = resolved.membership?.get(item.tag);
      // Only the hours the widget shows (its range, and the group's membership): an
      // axis dragged past the range shows nothing beyond it.
      const shownHours = [{ tag: item.tag, firstHour: extent[0], lastHour: extent[1] }];
      const lines = Object.fromEntries(MEASURES.map(measure => {
        const runs = clipRuns(percentRuns(item[measure.key], data.decimals), shownHours);
        return [measure.key, membership ? clipRuns(runs, membership) : runs];
      })) as Record<Measure, Run[]>;
      const valueAt = (measure: Measure, hour: number) => hour >= extent[0] && hour <= extent[1] && isMember(membership, hour) ? rawAt(item[measure], hour) : null;
      return { tag: item.tag, name: style?.name ?? track?.source.metadata.countries.find(country => country.tag === item.tag)?.name ?? item.tag,
        color: recorded, recordedColour: Boolean(style?.color), lines, valueAt, carried: item.carried };
    });
    // Only recorded colours are adjusted; a country without one stays grey, as the diagnostic says.
    const coloured = raw.filter(item => item.recordedColour);
    const adjusted = tokens.countryColours === 'fixed'
      ? nudgeApart(coloured.map(item => item.color), parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : coloured.map(item => item.color);
    return raw.map(item => ({ ...item, hex: toHex(item.recordedColour ? adjusted[coloured.indexOf(item)] : item.color) }));
  }, [data, known, resolved.membership, tokens, track, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps

  // Event markers (mock-up politics-2), for one country: each kind can be switched off.
  const kinds = useMemo(() => new Set(([['wars', 'markWars'], ['factions', 'markFactions'], ['government', 'markGovernment'], ['focus', 'markFocus']] as const)
    .filter(([, key]) => options[key] !== false).map(([kind]) => kind as MarkerKind)), [options.markWars, options.markFactions, options.markGovernment, options.markFocus]);
  // A widget showing only a breakdown draws no markers, so asks for none.
  const partsOnly = options.show === 'stability' || options.show === 'warSupport';
  const markerTag = resolved.tags.length === 1 && kinds.size && !partsOnly ? resolved.tags[0] : null;
  const markerGroups = useCountryGroups(runtime, track, active && markerTag != null && (kinds.has('wars') || kinds.has('factions')));
  const markerRequest = (product: string) => track && markerTag ? { sourceId: track.source.id, revision: track.source.revision, product, params: { branchId: track.branch.id, tag: markerTag } } : null;
  const markerParties = useProduct<PartyHistoryReply>(runtime.client, kinds.has('government') ? markerRequest('party-history') : null, active, validatePartyHistoryReply);
  const markerFocus = useProduct<FocusReply>(runtime.client, kinds.has('focus') ? markerRequest('national-focus') : null, active, validateFocusReply);
  // The idea history of the marker country itself (a group may resolve to another country).
  const markerIdeas = useProduct<IdeaHistoryReply>(runtime.client, kinds.has('government') ? markerRequest('idea-history') : null, active, validateIdeaHistoryReply);
  const markerSources = useMemo(() => !track || !markerTag ? null : {
    tag: markerTag, extent, branch: [track.branch.firstHour, track.branch.lastHour] as const,
    member: (hour: number) => isMember(resolved.membership?.get(markerTag), hour),
    groups: markerGroups.data, ideas: markerIdeas.data && markerIdeas.data.tag === markerTag ? markerIdeas.data : undefined,
    parties: markerParties.data, focus: markerFocus.data,
  }, [track, markerTag, extent[0], extent[1], resolved.membership, markerGroups.data, markerIdeas.data, markerParties.data, markerFocus.data]); // eslint-disable-line react-hooks/exhaustive-deps
  // Other countries are named as the game showed them at each event's hour; until
  // (or where) that is not known, by their name in the recording's metadata.
  const namesWanted = useMemo(() => {
    const wanted: NameAt[] = [];
    if (markerSources) stabilityMarkers({ ...markerSources, countryName: (other, hour) => { wanted.push({ hour, tag: other }); return other; } }, kinds);
    return wanted;
  }, [markerSources, kinds]);
  const namesAt = useCountryNamesAt(runtime.client, track, namesWanted, active);
  const badges = useMemo(() => !track || !markerSources ? [] : stabilityMarkers({ ...markerSources,
    countryName: (other, hour) => namesAt.get(nameKey(hour, other)) ?? track.source.metadata.countries.find(country => country.tag === other)?.name ?? other,
  }, kinds), [track, markerSources, namesAt, kinds]);
  // The marker sources shown report their notes and failures, so a missing kind of event is never silent.
  const markerNotes = useMemo(() => markerTag ? ([
    ['wars and factions', markerGroups, kinds.has('wars') || kinds.has('factions')], ['laws and advisors', markerIdeas, kinds.has('government')],
    ['ruling party', markerParties, kinds.has('government')], ['focuses', markerFocus, kinds.has('focus')],
  ] as const).flatMap(([what, state, used]) => {
    if (!used) return [];
    const product = state as { error?: string; data?: { warnings: string[] } };
    return product.error ? [`Markers for ${what} could not be loaded: ${product.error}`] : product.data?.warnings ?? [];
  }) : [], [markerTag, kinds, markerGroups.data, markerGroups.error, markerIdeas.data, markerIdeas.error, markerParties.data, markerParties.error, markerFocus.data, markerFocus.error]); // eslint-disable-line react-hooks/exhaustive-deps
  // The drill-down (mock-up drill-1): one country's parts, opened by a click on the
  // chart, or shown as the widget itself when pinned as a breakdown (the Show setting).
  const pinnedMeasure = options.show === 'stability' || options.show === 'warSupport' ? options.show as Measure : null;
  const [drill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  const [drillMeasure, setDrillMeasure] = useState<Measure>(pinnedMeasure ?? 'stability');
  // A changed Show setting shows its measure.
  useEffect(() => { if (pinnedMeasure) setDrillMeasure(pinnedMeasure); }, [pinnedMeasure]);
  const drillAnchor = useRef<HTMLDivElement>(null);
  // The one country the widget shows, known before its values load.
  const singleTag = resolved.tags.length === 1 ? resolved.tags[0] : null;
  useEffect(() => dispatchDrill({ type: 'close' }), [track?.source.id, track?.source.revision, track?.branch.id, singleTag]);
  const partsRequest = track && singleTag && (drill.breakdown || pinnedMeasure) ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'stability-parts', params: { branchId: track.branch.id, tag: singleTag } } : null;
  const parts = useProduct<StabilityPartsReply>(runtime.client, partsRequest, active, validateStabilityPartsReply);
  // Every diagnostic in one place, so no source's warning replaces another's.
  useEffect(() => {
    const uncoloured = series.filter(item => !item.recordedColour).map(item => item.name);
    const warnings = [...(data?.warnings ?? []), ...(styles.data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(styles.error ? [`Country names and colours could not be loaded: ${styles.error}`] : []),
      ...(uncoloured.length ? [`No map colour was recorded for ${uncoloured.join(', ')}; shown in grey.`] : []),
      ...(breakdown.data?.warnings ?? []), ...(breakdown.error ? [`The breakdown could not be loaded: ${breakdown.error}`] : []),
      ...markerNotes,
      ...(partsRequest ? [...(parts.data?.warnings ?? []), ...(parts.error ? [`The parts could not be loaded: ${parts.error}`] : [])] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, styles.data, styles.error, series, groups.data, breakdown.data, breakdown.error, markerNotes, partsRequest != null, parts.data, parts.error, onDiagnostics]);
  const menu = useContextMenu('Chart actions');
  const head = playhead ?? extent[1];
  // With the keyboard: while focus is in the widget, the breakdown of the read behind
  // the moment shown is loaded once playback rests there, and read out below.
  const [focused, setFocused] = useState(false);
  // Only while the country is shown there (a group counted at each hour may have left it).
  // ... and only where the hour shown was itself recorded (a read can cover an hour left out).
  const headRecorded = single != null && shown.some(measure => rawAt(single[measure.key], Math.floor(head)) != null);
  const headRead = single && headRecorded && isMember(resolved.membership?.get(single.tag), Math.floor(head)) ? readHourAt(single.carried, Math.floor(head)) : null;
  // Its own request, so a pointer resting on the chart never replaces it.
  const [spokenAt, setSpokenAt] = useState<{ tag: string; read: number; hour: number } | null>(null);
  // A new recording, branch or country asks afresh.
  useEffect(() => setSpokenAt(null), [track?.source.id, track?.source.revision, track?.branch.id, single?.tag]);
  const spokenRequest = track && focused && single && spokenAt && spokenAt.tag === single.tag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-stability', params: { branchId: track.branch.id, tags: [single.tag], hour: spokenAt.hour, breakdownOnly: true } } : null;
  const spoken = useProduct<CountryStabilityReply>(runtime.client, spokenRequest, active, validateStabilityReply);
  const spokenReply = spoken.data?.series[0];
  const spokenFor = single && spokenReply?.tag === single.tag && spokenReply.breakdown?.readHour === headRead ? spokenReply.breakdown : null;
  // An hour of the read that had no breakdown (a gap) is asked again at the hour shown.
  const spokenEmpty = spoken.data != null && spokenReply?.tag === single?.tag && (spokenReply?.breakdown == null || !spokenReply.breakdown.linesAvailable);
  useEffect(() => {
    if (!focused || !single || headRead == null) return;
    const asked = spokenAt?.tag === single.tag && spokenAt.read === headRead;
    if (asked && !(spokenEmpty && spokenAt.hour !== Math.floor(head))) return;
    const timer = setTimeout(() => setSpokenAt({ tag: single.tag, read: headRead, hour: Math.floor(head) }), FOCUS_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [focused, single, headRead, head, spokenAt, spokenEmpty]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const layers = useMemo<ChartLayer[]>(() => series.flatMap(item => shown.map(measure => ({
    draw: (ctx, frame, chartTokens, state) => { drawLine(ctx, frame, chartTokens, item.lines[measure.key], item.hex, { dash: measure.dash, until: state.playhead ?? undefined }); },
    extent: (x: readonly [number, number]) => extentOf(item.lines[measure.key], x[0], Math.min(x[1], until ?? x[1])),
  }))), [series, shown, until]);

  if (!track) return <WidgetState kind="empty" icon="sliders" message="Load a campaign to see stability and war support." />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (widget.countries.kind === 'group' && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  // A pinned breakdown is the widget itself: the main chart's settings do not apply.
  if (pinnedMeasure) {
    if (!singleTag) return <WidgetState kind="empty" message="Choose one country to see what makes up its values." />;
    if (parts.error) return <WidgetState kind="error" message={parts.error} onRetry={parts.retry} />;
    // The values' reads say which parts were carried from an earlier read: needed too.
    if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
    if (!parts.data || !data) return <WidgetState kind="loading" label="Loading what makes up the values…" />;
    const carried = data.series.find(item => item.tag === singleTag)?.carried;
    return <div className="stability is-pinned">
      <StabilityDrill reply={parts.data} measure={pinnedMeasure} playback={{ playhead, mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }} hour={Math.floor(head)} extent={extent}
        within={resolved.membership?.get(singleTag) ?? null} readAt={carried ? hour => readHourAt(carried, hour) : undefined}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])}
        branchStart={track.branch.firstHour} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} tokens={tokens} />
      {menu.menu}
    </div>;
  }
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label="Loading stability and war support…" />;
  if (!shown.length) return <WidgetState kind="empty" message="Both lines are switched off in the settings." />;
  if (series.every(item => shown.every(measure => !item.lines[measure.key].length))) return <WidgetState kind="unavailable" message="No stability or war support was recorded for these countries in this branch." />;

  const decimals = data.decimals;
  // Following with Live off, the breakdown shows the moment shown: the end of the view.
  const drillAt = drillHour(drill, playhead ?? extent[1]);
  // Zoom fits the values drawn in view (both bounds, as the view moves); the fixed scale is 0-100%.
  const zoom = options.scale === 'zoom';
  // The breakdown of the moment shown, every line, when it has been loaded for the keyboard.
  const spokenBreakdown = spokenFor ? shown.map(measure =>
    breakdownRows(spokenFor, measure.key, decimals, measure.label, Number.MAX_SAFE_INTEGER).rows
      .map(row => row.value ? `${row.label} ${row.value}` : row.label).join(', ')).join('; ') : '';
  const spokenNote = !focused || headRead == null ? ''
    : spokenBreakdown ? `Breakdown on ${dayLabel(Math.floor(head))}${headRead < Math.floor(head) ? `, as read at ${formatDateTime(headRead)}` : ''}: ${spokenBreakdown}.`
      : spoken.error && spokenAt?.read === headRead ? 'The breakdown could not be loaded.'
        : spokenEmpty && spokenAt?.read === headRead && spokenAt.hour === Math.floor(head) ? `No breakdown was recorded for ${dayLabel(Math.floor(head))}.` : '';
  const main = <div className="stability-main">
    <Legend label="Countries" items={series.map(item => ({ id: item.tag, label: item.name, color: item.hex,
      flag: <SourceFlag client={runtime.client} source={track.source} tag={item.tag} size="shield" /> }))} />
    {/* The line styles are named, as signed off: never the dash pattern alone. */}
    <p className="stability-key">{shown.map(measure => <span key={measure.key}><i aria-hidden="true" className={measure.dash ? 'is-dashed' : undefined} />{measure.dash ? 'Dashed' : 'Solid'}: {measure.label.toLowerCase()}</span>)}</p>
    {/* The current values as text, for keyboard and screen-reader users. */}
    <p className="sr-only" aria-live="off">{`${dayLabel(Math.floor(head))}: `}{series.map(item => {
      // Values carried from an earlier read say when they were read, as the tooltip does.
      const read = readHourAt(item.carried, Math.floor(head));
      const values = shown.map(measure => {
        const raw = item.valueAt(measure.key, Math.floor(head));
        return `${measure.label} ${raw == null ? 'not recorded' : formatPercent(raw, decimals)}`;
      });
      const carried = read < Math.floor(head) && shown.some(measure => item.valueAt(measure.key, Math.floor(head)) != null);
      return `${item.name}: ${values.join(', ')}${carried ? ` (as read at ${formatDateTime(read)})` : ''}`;
    }).join('; ')}</p>
    {/* Announced when it loads for a keyboard user, as focus stays on the chart. */}
    <p className="sr-only" aria-live="polite">{spokenNote}</p>
    {/* Leaving the chart ends a hover, so coming back to the same hour asks again for a failed breakdown. */}
    <div className="stability-chart" onPointerLeave={() => { tipHour.current = null; }}>
      <TimeChart label={`Stability and war support of ${series.map(item => item.name).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: zoom ? null : 0, yMax: zoom ? null : 100, log: false }} fitValues={zoom ? zoomRange : undefined}
        padding={{ right: options.endLabels ? CHART_LAYOUT.countryEndLabels : CHART_LAYOUT.right }} layers={layers} active={active && !(drill.breakdown && options.drill !== 'popout')}
        revision={`${head}:${data.branchId}:${series.length}:${shown.length}:${viewport}:${breakdown.data ? breakdownFor?.readHour ?? 'empty' : breakdown.error ? 'failed' : 'none'}`}
        formatValue={value => `${Number.isInteger(value) ? value : value.toFixed(1)}%`}
        badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hovered) => {
          const rows: TooltipRow[] = [...badgeRows(hovered)];
          if (series.length === 1) {
            const item = series[0];
            const read = readHourAt(item.carried, hour);
            // Ask for the breakdown of the read behind this hour; the chart redraws when it arrives.
            const key = `${item.tag}:${read}`;
            const asked = requestHours.current.get(key);
            // A read whose asked hour was a gap is asked again at this (shown) hour.
            const retry = asked != null && asked !== hour && emptyHours.current.has(`${key}:${asked}`) && shown.some(measure => item.valueAt(measure.key, hour) != null);
            if (asked == null || retry) requestHours.current.set(key, hour);
            if (hoverKey.current !== key || retry) {
              hoverKey.current = key;
              const at = requestHours.current.get(key)!;
              queueMicrotask(() => setHover({ tag: item.tag, read, hour: at }));
            }
            // Once the attempts are spent, moving to another hour of the read asks afresh
            // (a tooltip rebuilt for the same hour is no new hover).
            const moved = tipHour.current !== hour;
            tipHour.current = hour;
            if (moved && breakdown.error && hoverRead === read && retries.current >= BREAKDOWN_RETRIES) {
              retries.current = 0;
              queueMicrotask(retryBreakdown.current);
            }
            // The reply for this read: loaded (with or without a breakdown), failed, or on its way.
            const current = breakdown.data && hoverRead === read;
            const ready = current && breakdownFor ? breakdownFor : null;
            let recorded = false;
            for (const [index, measure] of shown.entries()) {
              const raw = item.valueAt(measure.key, hour);
              if (raw == null) { rows.push({ label: measure.label, value: 'Not recorded', tone: 'muted', strong: true, divider: index > 0 || rows.length > 0 }); continue; }
              if (!ready) {
                rows.push({ label: measure.label, value: formatPercent(raw, decimals), strong: true, divider: index > 0 || rows.length > 0 });
                continue;
              }
              const built = breakdownRows(ready, measure.key, decimals, measure.label, linesPerMeasure(shown.length));
              recorded ||= built.recorded;
              rows.push(...built.rows.map((row, position) => position === 0 ? { ...row, divider: index > 0 || rows.length > 0 } : row));
            }
            const notes = [
              ...(breakdown.error && hoverRead === read ? ['The breakdown could not be loaded.']
                : current && !breakdownFor ? ['No breakdown was recorded for this hour.']
                  : !ready ? ['Loading the breakdown…'] : []),
              // Values carried from an earlier read say when they were read.
              ...(read < hour ? [`As read at ${formatDateTime(read)}.`] : []),
              ...(recorded && ready ? [`Grey lines are the game's own text (${ready.language}); their source is not identified.`] : []),
              'Click for the full breakdown.',
            ];
            return { title: dayLabel(hour), rows, note: notes.length ? notes.join(' ') : undefined };
          }
          // One row per country with both values; shared fitting keeps the full read-out.
          const carried: number[] = [];
          for (const item of series) {
            const values = shown.map(measure => {
              const raw = item.valueAt(measure.key, hour);
              return `${measure.dash ? 'WS' : 'S'} ${raw == null ? '—' : formatPercent(raw, decimals)}`;
            });
            const none = shown.every(measure => item.valueAt(measure.key, hour) == null);
            const read = readHourAt(item.carried, hour);
            const early = read < hour && !none;
            if (early) carried.push(read);
            rows.push({ label: item.name, swatch: item.hex, value: none ? 'Not recorded' : `${values.join(' · ')}${early ? ' †' : ''}`, tone: none ? 'muted' : undefined, short: true });
          }
          // Values carried from an earlier read are marked; the note stays one line however many there are.
          const first = Math.min(...carried);
          const last = Math.max(...carried);
          const note = [`${shown.map(measure => `${measure.dash ? 'WS' : 'S'}: ${measure.label.toLowerCase()}`).join(', ')}.`,
            ...(carried.length ? [first === last ? `† As read at ${formatDateTime(first)}.` : `† As read earlier, between ${formatDateTime(first)} and ${formatDateTime(last)}.`] : []),
            'Choose one country to see what makes up its values.'];
          return fitTooltip([{ title: dayLabel(hour), rows, note: note.join(' ') }], viewport);
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          // No line ends in view (the view lies wholly after them): no labels.
          if (at < frame.x[0]) return;
          const ends = series.flatMap(item => shown.flatMap(measure => {
            // Each line's own last sample in view: a line may end before the others.
            const last = lineEndHour(item.lines[measure.key], frame.x[0], at);
            if (last == null) return [];
            const raw = item.valueAt(measure.key, last);
            if (raw == null) return [];
            const text = `${series.length > 1 ? `${item.name}: ` : ''}${measure.label} ${formatPercent(raw, decimals)}`;
            return [{ id: `${item.tag}:${measure.key}`, y: frame.toY(Number(raw) / 10 ** decimals * 100), text, color: item.hex }];
          }));
          // Only as many labels as the plot can hold: war support's are dropped first, then all
          // (the legend and tooltip still name every line).
          const room = Math.floor((frame.plotHeight + END_LABEL_GAP) / (END_LABEL_HEIGHT + END_LABEL_GAP));
          const fitted = ends.length <= room ? ends : ends.filter(end => end.id.endsWith(':stability'));
          if (fitted.length > room) return;
          ends.splice(0, ends.length, ...fitted);
          placeEndLabels(ctx, frame, chartTokens, ends, END_LABEL_GAP);
        }}
        onPick={single ? hour => {
          // Opening with one line shown opens that line's parts.
          if (!drill.breakdown && shown.length === 1) setDrillMeasure(shown[0].key);
          dispatchDrill(drill.breakdown ? { type: 'pick', hour }
            : { type: 'open', breakdown: 'parts', hour, playing: runtime.clock.getSnapshot().playing, playhead });
        } : undefined}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
  </div>;
  const breakdownOf = STABILITY_BREAKDOWNS.find(item => item.id === drillMeasure) ?? STABILITY_BREAKDOWNS[0];
  return <div className="stability" ref={drillAnchor} onFocus={() => setFocused(true)}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false); }}>
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown != null} anchor={drillAnchor}
      title={`What makes up ${MEASURE_NAMES[drillMeasure].toLowerCase()}`} day={drillAt == null ? '' : dayLabel(Math.floor(drillAt))}
      following={drill.follow} canFollow={playhead != null} onClose={() => {
        dispatchDrill({ type: 'close' });
        // Focus goes back to the chart (once it is no longer inert) when the breakdown's own
        // controls closed it, so the keyboard keeps its place; a click elsewhere keeps its focus.
        requestAnimationFrame(() => {
          const focused = document.activeElement;
          const lost = !focused || focused === document.body;
          if (!lost && !drillAnchor.current?.contains(focused) && !focused.closest('.drill-popout')) return;
          drillAnchor.current?.querySelector<HTMLCanvasElement>('.stability-chart canvas')?.focus({ preventScroll: true });
        });
      }} onFollow={() => dispatchDrill({ type: 'follow' })}
      onPin={() => runtime.addWidgetBelow(widget.id, pinnedBreakdown(widget, breakdownOf, track.id))} main={main}>
      {parts.error ? <WidgetState kind="error" message={parts.error} onRetry={parts.retry} />
        : !parts.data ? <WidgetState kind="loading" label="Loading what makes up the values…" />
          : <StabilityDrill reply={parts.data} measure={drillMeasure} onMeasure={setDrillMeasure} playback={{ playhead, mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }} hour={drillAt} onPick={hour => dispatchDrill({ type: 'pick', hour })}
            within={single ? resolved.membership?.get(single.tag) ?? null : null} readAt={single ? hour => readHourAt(single.carried, hour) : undefined}
            onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])}
            extent={extent} branchStart={track.branch.firstHour} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} tokens={tokens} />}
    </DrillHost>
    {menu.menu}
  </div>;
}
