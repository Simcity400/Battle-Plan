import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { drawLineSummary } from '../../charts/chartSummary';
import type { ChartSummary } from '../../charts/chartSummary';
import { plotFrame } from '../../charts/frame';
import { jumpItem } from '../../charts/jump';
import { categoricalPalette, SECONDARY_DASH } from '../../charts/palette';
import Timeline from '../../charts/timeline/Timeline';
import { laneRequests } from '../../charts/timeline/laneRequests';
import type { LaneGeometry, TimelineLane } from '../../charts/timeline/Timeline';
import type { ChartTokens } from '../../charts/tokens';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { motionOf } from '../../charts/easing.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { followStore } from '../../dashboard/follow/store.ts';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { followRefusal, followsHead, useFollow } from '../../dashboard/follow/useFollow.ts';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import { useHeadHour } from '../army-size/headHour';
import { useIdentityKey } from '../../dashboard/atHead';
import { validateMeasureStates } from '../country-measure/measureSeriesData';
import type { MeasureStates } from '../country-measure/measureSeriesData';
import { useCountriesAt } from '../../dashboard/useCountriesAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { mix } from '../../shell/appearance/color';
import { useMeasureSeries } from '../country-measure/measureSeries';
import { validateSubjectReply } from '../subjects/subjectsData';
import { autonomyLanes, formatProgress, formatScore, headKey, holderSince, laneIdentity, laneLabel, levelAt, levelName, levelOrder, runAt } from './autonomyData';
import type { AutonomyLane } from './autonomyData';
import { OPTIONS } from './definition';
import './autonomy.css';
import type { HourSpan, SubjectReply } from '../../generated/dashboard';

const NO_WARNINGS: string[] = [];
const NO_SPANS: HourSpan[] = [];
const LANE_HEIGHT = 54;
const LANE_PAD = 8;
const NO_PAINT = () => {};

function paintLane(ctx: CanvasRenderingContext2D, lane: AutonomyLane, geometry: LaneGeometry, tokens: ChartTokens, colour: (level: string | null) => string, summary: ChartSummary | undefined): void {
  const top = geometry.top + LANE_PAD;
  const height = geometry.height - 2 * LANE_PAD;
  const bottom = top + height;
  const end = geometry.playhead == null ? Infinity : geometry.playhead;
  const toY = (value: number) => bottom - Math.max(0, Math.min(1, value)) * height;
  const frame = plotFrame(geometry.right, geometry.top + geometry.height, { left: geometry.left, right: 0, top, bottom: LANE_PAD }, geometry.hours, [0, 1], false);
  for (const span of lane.spans) {
    const to = Math.min(span.lastHour + 1, end);
    if (span.firstHour >= to) continue;
    const x0 = geometry.toX(span.firstHour);
    ctx.fillStyle = mix(tokens.surface, colour(span.level), 0.3);
    ctx.fillRect(x0, top, geometry.toX(to) - x0, height);
  }
  ctx.save();
  ctx.setLineDash(SECONDARY_DASH); ctx.lineWidth = 1; ctx.strokeStyle = mix(tokens.surface, tokens.text, 0.4);
  for (const stretch of lane.within) {
    const to = Math.min(stretch.lastHour + 1, end);
    if (stretch.firstHour >= to) continue;
    for (const y of [Math.round(top) + 0.5, Math.round(bottom) - 0.5]) {
      ctx.beginPath(); ctx.moveTo(geometry.toX(stretch.firstHour), y); ctx.lineTo(geometry.toX(to), y); ctx.stroke();
    }
  }
  ctx.restore();
  ctx.save();
  ctx.strokeStyle = tokens.text; ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round'; ctx.lineCap = 'round';
  if (summary) drawLineSummary(ctx, { ...frame, toY }, tokens, summary, tokens.text, false, end === Infinity ? null : end, { endDot: false });
  for (const hour of lane.changes) {
    if (hour > end) continue;
    ctx.beginPath(); ctx.arc(geometry.toX(hour), top + height / 2, 4, 0, Math.PI * 2);
    ctx.fillStyle = tokens.accent; ctx.fill();
    ctx.lineWidth = 1.5; ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  ctx.restore();
}

export default function AutonomyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const followsClock = widget.time.mode === 'follow';
  const structure = useRef<(hour: number) => string>(() => '');
  const playbackHour = useHeadHour(clock, active && followsClock, hour => structure.current(hour));
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const bySubject = options.countriesAre === 'subjects';
  const playhead = window && following ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const head = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'subjects', params: { branchId: track.branch.id } } : null;
  const product = useProduct<SubjectReply>(runtime.client, request, active, validateSubjectReply, track?.source);
  const data = product.data;
  const filter = useMemo(() => worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, head, lastHour),
    [widget.countries, fallback, groups.data, head, lastHour]);
  const lanesData = useMemo(() => data ? autonomyLanes(data, filter.tags, bySubject) : [], [data, filter.tags, bySubject]);
  const tags = useMemo(() => lanesData.map(lane => lane.tag), [lanesData]);
  const within = useMemo(() => new Map(lanesData.map(lane => [lane.tag, lane.within])), [lanesData]);
  const series = useMeasureSeries(runtime.client, track?.source, track?.branch.id, 'autonomy', 'progress', tags,
    following && !followsClock ? head : null, hoverHour, active, false, within, lastHour, followsClock);
  const axisOffset = track?.axisOffset ?? 0;
  const tilesLanded = series.subscribe;
  const liveHead = useMemo(() => followsClock && track ? { read: () => clock.getSnapshot().position + axisOffset, subscribe: (changed: () => void) => {
    const stops = [clock.subscribeFrame(changed), tilesLanded(changed)];
    return () => { for (const stop of stops) stop(); };
  } } : undefined, [followsClock, track, clock, axisOffset, tilesLanded]);
  const store = useMemo(() => track ? followStore(runtime.client, track.source, clock, track.axisOffset) : null, [runtime.client, clock, track?.source, track?.axisOffset]);
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const headQuery = useMemo(() => followsClock && active && track && tags.length ? { product: 'series-state', params: { branchId: track.branch.id,
    family: { kind: 'autonomy', measure: 'progress' }, reads: tags.map(tag => ({ country: countryFromKey(tag), hour: 0 })) } } : null, [followsClock, active, track?.branch.id, tags]);
  const followed = useFollow(store, headQuery, root, NO_PAINT);
  const entry = followsHead(followed) ? followed.entry : null;
  const refused = followRefusal(followed);
  const places = series.decimals;
  useEffect(() => {
    const stops = entry && places !== null ? tags.map((tag, row) => followAtHead(series.head(tag), () => rowAmount(entry.answer, row, places))) : [];
    return () => stops.forEach(stop => stop());
  }, [entry, places, tags, series.head]);
  useShownEachFrame(clock, () => data ? liveHead ? Math.min(liveHead.read(), lastHour) : head : null);
  const hovered = hoverHour === null ? null : Math.floor(hoverHour);
  const scoreRequest = (measure: string) => track && hovered !== null && tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'series-state',
    params: { branchId: track.branch.id, family: { kind: 'autonomy', measure }, reads: tags.map(tag => ({ country: countryFromKey(tag), hour: hovered })) } } : null;
  const scores = useProduct<MeasureStates>(runtime.client, scoreRequest('score'), active, validateMeasureStates, track?.source);
  const lowers = useProduct<MeasureStates>(runtime.client, scoreRequest('lower'), active, validateMeasureStates, track?.source);
  const uppers = useProduct<MeasureStates>(runtime.client, scoreRequest('upper'), active, validateMeasureStates, track?.source);
  const scoreAt = (states: MeasureStates | undefined, tag: string, hour: number) => {
    const state = states?.states.find(item => countryKey(item.country) === tag && item.hour === hour);
    return state ? formatScore(state.run?.valueRaw, states?.decimals, state.missing) : 'Loading…';
  };
  const order = useMemo(() => levelOrder(data?.levels ?? []), [data]);
  const palette = useMemo(() => categoricalPalette(tokens.surface), [tokens.surface]);
  const neutral = mix(tokens.surface, tokens.text, 0.25);
  const colour = useCallback((level: string | null) => {
    const at = level == null ? undefined : order.get(level);
    return at ? palette[at.index % palette.length] : neutral;
  }, [order, palette, neutral]);
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const onLanesInView = useCallback((from: number, to: number) => setInView([from, to]), []);
  const flagTags = useMemo(() => laneRequests(tags, inView).looks, [tags, inView]);
  const holders = useMemo(() => groups.data ? new Map(groups.data.countryIdentities.map(history => [history.tag, history.identities])) : null, [groups.data]);
  const segmentAt = useCallback((tag: string, hour: number) => holderSince(holders, tag, hour) ?? null, [holders]);
  const { country, revision } = useCountriesAt(runtime.client, track, flagTags, head, active, tags, holders ? segmentAt : undefined, undefined,
    { clock, on: followsClock, hour: (clockHour: number) => track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null });
  const laneName = (tag: string) => laneIdentity(tag, holderSince(holders, tag, head), country(tag));
  const identityKey = useIdentityKey(runtime.client, track, active && followsClock);
  const hover = useHoverLabels(runtime.client, track, active, country);
  const progressAt = (tag: string, hour: number) => {
    const line = series.lines.find(item => item.tag === tag);
    return line?.runs.find(run => run.firstHour <= hour && hour <= run.lastHour)?.valueRaw ?? null;
  };
  const lineOf = series.lineOf;
  const unobserved = data?.unobserved ?? NO_SPANS;
  structure.current = hour => data ? `${headKey(lanesData, data.levels, Math.floor(hour), unobserved)}|${identityKey(Math.floor(hour), tags) ?? ''}` : '';

  const menu = useContextMenu('Timeline actions');
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(groups.error ? [`Country identity changes could not be loaded: ${groups.error.message}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, onDiagnostics]);

  const lanes = useMemo<TimelineLane[]>(() => !data ? [] : lanesData.map(lane => {
    const labels = laneLabel(lane, data.levels, head, unobserved);
    const level = levelAt(lane, head);
    const named = laneName(lane.tag);
    return {
      id: lane.tag, label: labels.label, spans: lane.spans.map(span => ({ id: span.id, firstHour: span.firstHour, lastHour: span.lastHour, label: levelName(data.levels, span.level) })),
      identity: named,
      pill: { text: labels.pill, color: level !== undefined ? colour(level) : neutral },
      muted: level === undefined,
      paint: (ctx, geometry, chartTokens) => paintLane(ctx, lane, geometry, chartTokens, colour, lineOf(lane.tag)),
    };
  }), [data, lanesData, head, unobserved, country, holders, colour, neutral, lineOf, series.held]);

  const groupFailure = groups.error && widget.countries.kind === 'group' ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  const content = (() => {
    if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see its subjects’ autonomy." />;
    if (refused) return <WidgetState kind="unavailable" message={refused} />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
    if (groupFailure && !groups.data) return groupFailure;
    if (widget.countries.kind === 'group' && !groups.data) return <WidgetState kind="loading" label="Loading country membership…" />;
    if (filter.reason) return <WidgetState kind="empty" message={filter.reason} />;
    if (product.error) return <WidgetState kind="error" failure={product.error} onRetry={product.retry} />;
    if (series.error) return <WidgetState kind="error" failure={series.error} onRetry={series.retry} />;
    if (!data) return <WidgetState kind="loading" label="Loading autonomy…" />;
    if (!lanesData.length) return <WidgetState kind="empty" message={data.runs.length ? 'None of the chosen countries has been a subject.' : data.warnings.length ? 'Subject relations are unavailable.' : 'No subjects recorded.'} />;
    const description = `${dayLabel(head)}: ${lanesData.map(lane => `${laneName(lane.tag).label}: ${laneLabel(lane, data.levels, head, unobserved).label}`).join('; ')}.`;
    return <div className="autonomy" ref={setRoot}>
      <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label={`Autonomy of ${lanesData.map(lane => laneName(lane.tag).label).join(', ')}`} extent={extent} playhead={playhead} liveHead={liveHead} labelHour={followsClock ? undefined : head} preserveZoomOnExtentChange={options.grow === true} lanes={lanes}
        rowHeight={LANE_HEIGHT} showIcons={false} showNames={false} description={description} hoursPerSecond={motionOf(clock.getSnapshot())} unobserved={unobserved}
        active={active} revision={`${head}:${data.branchId}:${revision}:${hover.revision}:${scores.data ? 1 : 0}${lowers.data ? 1 : 0}${uppers.data ? 1 : 0}`}
        onHover={setHoverHour} onLanesInView={onLanesInView} onSummaryView={view => series.onSummaryView({ ...view, end: view.to })}
        tooltip={(lane, _span, hour) => {
          const about = lanesData.find(item => item.tag === lane.id);
          if (!about) return null;
          const run = runAt(about, hour);
          hover.ask(hour, [about.tag, ...(run ? [run.overlord] : [])]);
          const named = hover.labelAt(hour);
          const label = named(about.tag);
          if (!run) return { title: `${label.name} · ${dayLabel(hour)}`, countryFlag: label, rows: [{ label: 'Autonomy', value: laneLabel(about, data.levels, hour, unobserved).label, tone: 'muted' }] };
          const level = levelAt(about, hour) ?? null;
          const place = level == null ? undefined : order.get(level);
          const overlord = named(run.overlord);
          const rows: TooltipRow[] = [
            { label: 'Subject level', value: levelName(data.levels, level), swatch: colour(level) },
            { label: 'Overlord', value: overlord.name, countryFlag: overlord },
            { label: place?.highest ? 'Progress within the level' : 'Progress to the next level', value: formatProgress(progressAt(about.tag, Math.floor(hour))) },
            { label: 'Autonomy score', value: scoreAt(scores.data, about.tag, Math.floor(hour)), divider: true },
            { label: 'Moves down below', value: `${scoreAt(lowers.data, about.tag, Math.floor(hour))}${place?.lowest ? ' · no lower level' : ''}` },
            { label: 'Moves up at', value: `${scoreAt(uppers.data, about.tag, Math.floor(hour))}${place?.highest ? ' · no higher level' : ''}` },
          ];
          return { title: `${label.name} · ${dayLabel(hour)}`, countryFlag: label, rows, note: contentTitleHint(level ?? undefined) };
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
      {menu.menu}
    </div>;
  })();
  return <>{groups.data && groupFailure}{content}</>;
}
