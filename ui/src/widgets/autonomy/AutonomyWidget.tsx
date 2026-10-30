import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { dayLabel, tracePath } from '../../charts/draw';
import { drawApproximateSpans, drawLineSummary } from '../../charts/chartSummary';
import type { ChartSummary } from '../../charts/chartSummary';
import { plotFrame } from '../../charts/frame';
import { jumpItem } from '../../charts/jump';
import { categoricalPalette, SECONDARY_DASH } from '../../charts/palette';
import { curveSegments, runAt, spansCovered } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import { laneRequests } from '../../charts/timeline/laneRequests';
import type { LaneGeometry, TimelineLane } from '../../charts/timeline/Timeline';
import type { ChartTokens } from '../../charts/tokens';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { PreparedKey } from '../../charts/PreparedKey';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { contentTitleHint } from '../../dashboard/contentTitles';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupRuns, groupUnobserved, MEMBERSHIP_PREPARING, worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { usePreparedWindow } from '../../dashboard/preparedSeries';
import { useParityRows } from '../../dashboard/parityRows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { inSpans } from '../../charts/timeline/layout';
import { mergePreparedAutonomy, overlayReachedAutonomy, validatePreparedAutonomy } from './preparedAutonomy';
import { mix } from '../../shell/appearance/color';
import { absentAt, autonomyUnprepared, autonomyViews, chosenViews, clipSubject, formatProgress, formatScore, laneLabels, levelName, membershipUnknownFor, noLanesMessage, overlordIdentityUnknownFor, runOf, subjectLane,
  } from './autonomyData';
import type { SubjectLane } from './autonomyData';
import { OPTIONS } from './definition';
import './autonomy.css';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
const NO_SPANS: { firstHour: number; lastHour: number }[] = [];
/** A lane's height. */
const LANE_HEIGHT = 54;
/** Room between a lane's edge and its thresholds. */
const LANE_PAD = 8;

/** One subject's lane: its level over time as coloured bands,
 * the move-up and move-down scores as the dashed top and bottom edges, its
 * progress between them as a line, and a dot where the level changed. */
function paintLane(ctx: CanvasRenderingContext2D, lane: SubjectLane, geometry: LaneGeometry, tokens: ChartTokens, colour: (span: string) => string, summary: ChartSummary | undefined, approximate: ChartSummary['approximate']): void {
  const top = geometry.top + LANE_PAD;
  const height = geometry.height - 2 * LANE_PAD;
  const bottom = top + height;
  const end = geometry.playhead == null ? Infinity : geometry.playhead;
  const toY = (value: number) => bottom - Math.max(0, Math.min(1, value)) * height;
  const frame = plotFrame(geometry.right, geometry.top + geometry.height, { left: geometry.left, right: 0, top, bottom: LANE_PAD }, geometry.hours, [0, 1], false);
  drawApproximateSpans(ctx, frame, approximate, end === Infinity ? null : end, () => {
  // Each level's colour, held for the whole span it applied.
  for (const span of lane.spans) {
    const to = Math.min(span.lastHour + 1, end);
    if (span.firstHour >= to) continue;
    const x0 = geometry.toX(span.firstHour);
    ctx.fillStyle = mix(tokens.surface, colour(span.id), 0.3);
    ctx.fillRect(x0, top, geometry.toX(to) - x0, height);
  }
  // The thresholds, along every recorded stretch.
  ctx.save();
  ctx.setLineDash(SECONDARY_DASH); ctx.lineWidth = 1; ctx.strokeStyle = mix(tokens.surface, tokens.text, 0.4);
  for (const stretch of lane.stretches) {
    const to = Math.min(stretch.lastHour + 1, end);
    if (stretch.firstHour >= to) continue;
    for (const y of [Math.round(top) + 0.5, Math.round(bottom) - 0.5]) {
      ctx.beginPath(); ctx.moveTo(geometry.toX(stretch.firstHour), y); ctx.lineTo(geometry.toX(to), y); ctx.stroke();
    }
  }
  ctx.restore();
  // Progress between them; a score past either threshold is drawn at that edge (the tooltip gives it exactly).
  const last = Math.min(geometry.hours[1], end);
  ctx.save();
  ctx.strokeStyle = tokens.text; ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round'; ctx.lineCap = 'round';
  if (summary && !summary.runs) drawLineSummary(ctx,
    { ...frame, toY: value => toY(value / 1e9) }, tokens, { ...summary, approximate: undefined }, tokens.text, false, end === Infinity ? null : end, { endDot: false });
  // A summary asked for a moment ago (the view moved on) still says the resolution is fine.
  const fine = summary?.runs && summary.width === Math.ceil(geometry.right - geometry.left)
    && summary.to <= last && summary.to >= geometry.hours[0];
  for (const { points, path } of curveSegments(fine ? lane.progress : [], geometry.hours[0], last, geometry.toX, toY)) {
    ctx.beginPath(); tracePath(ctx, path);
    if (points.length === 1) ctx.arc(points[0].x, points[0].y, tokens.lineWidth.series / 2, 0, Math.PI * 2);
    ctx.stroke();
  }
  // A dot where the level changed.
  for (const hour of lane.changes) {
    if (hour > end) continue;
    ctx.beginPath(); ctx.arc(geometry.toX(hour), top + height / 2, 4, 0, Math.PI * 2);
    ctx.fillStyle = tokens.accent; ctx.fill();
    ctx.lineWidth = 1.5; ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  ctx.restore();
  });
}

/** Each subject's autonomy over time: one lane per subject with
 * its flag and name, its current level as a coloured pill underneath, and the
 * playhead through every lane. Level names and order come from the recording's
 * content (its autonomy states); level colours are the palette in that order. */
export default function AutonomyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryTag(track);
  const bySubject = options.countriesAre === 'subjects';
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const head = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const prepared = usePreparedWindow(runtime.client, track?.source, 'autonomy', { branchId: track?.branch.id, overlords: [], subjects: [] },
    extent, [head, ...(hoverHour === null ? [] : [hoverHour])], active, validatePreparedAutonomy,mergePreparedAutonomy,overlayReachedAutonomy);
  const { data, retry } = prepared;
  const error = useHeldFailure(prepared.error, data !== undefined, retry);
  const approximate = useMemo(() => data?.approximate ?? Object.values(data?.summaries ?? {}).flatMap(summary => summary.approximate ?? []), [data]);
  const pending = data?.pending ?? NO_SPANS;
  const identityRows = data?.identities ?? groups.data?.countryIdentities;
  const identities = useMemo(() => identityRows ? countryIdentityIndex(identityRows) : null, [identityRows]);
  const identityHistories = useMemo(() => new Map(identityRows?.map(country => [country.tag, country.identities]) ?? []), [identityRows]);
  const root = useRef<HTMLDivElement>(null);
  const missingNativeSubjects = useMemo(() => data?.metadata.subjects.filter(subject => !spansCovered(subject.runs, identityHistories.get(subject.tag) ?? [])).length ?? 0,
    [data, identityHistories]);
  const filter = useMemo(() => worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, head, lastHour, identities),
    [widget.countries, fallback, groups.data, head, lastHour, identities]);
  // Hourly groups choose every member's history, then clip it at membership hours.
  const selection = useMemo(() => worldFilter(widget.countries.kind === 'group' && widget.countries.membership === 'hour'
    ? { ...widget.countries, membership: 'ever' } : widget.countries, fallback ? [fallback] : [], groups.data, head, lastHour, identities),
  [widget.countries, fallback, groups.data, head, lastHour, identities]);
  const hourlyGroup = widget.countries.kind === 'group' && widget.countries.membership === 'hour';
  const membership = useMemo(() => {
    if (!hourlyGroup || widget.countries.kind !== 'group' || !groups.data || !identities) return null;
    const byCountry = new Map<string, { firstHour: number; lastHour: number }[]>();
    for (const span of identities.membershipRuns(groupRuns(groups.data, widget.countries.group) ?? [])) {
      const key = identities.keyAt(span.tag, span.firstHour)!;
      const spans = byCountry.get(key);
      if (spans) spans.push(span); else byCountry.set(key, [span]);
    }
    for (const spans of byCountry.values()) spans.sort((a, b) => a.firstHour - b.firstHour);
    return byCountry;
  }, [hourlyGroup, widget.countries, groups.data, identities]);

  // A group counted at each hour shows each subject only while it (or its overlord) belongs,
  // and hours whose membership was not recorded as unknown. A group counted at one moment
  // chose its countries there: its unrecorded hours elsewhere hide nothing.
  const unobserved = useMemo(() => hourlyGroup && widget.countries.kind === 'group' && groups.data
    ? groupUnobserved(groups.data, widget.countries.group) : NO_SPANS, [hourlyGroup, widget.countries, groups.data]);
  const views = useMemo(() => data && identities ? autonomyViews(data.metadata, identities) : [], [data, identities]);
  const missingNativeOverlords = useMemo(() => !bySubject && selection.countryIds != null && identities
    ? views.filter(subject => overlordIdentityUnknownFor(subject, identities)).length : 0, [views, bySubject, selection.countryIds, identities]);
  const chosen = useMemo(() => selection.countryIds && identities ? chosenViews(views, selection.countryIds, bySubject, identities) : views,
    [views, selection.countryIds, bySubject, identities]);
  const subjects = useMemo(() => chosen.map(subject => clipSubject(subject, bySubject, membership, unobserved, identities ?? undefined)).filter(subject => subject.runs.length),
    [chosen, bySubject, membership, unobserved, identities]);
  const exactLanes = useMemo(() => new Map(data && identities ? autonomyViews(data, identities).map(subject => {
    const lane = subjectLane(clipSubject(subject, bySubject, membership, unobserved, identities), data.levels); return [lane.id, lane] as const;
  }) : []), [data, identities, bySubject, membership, unobserved]);
  const lanesData = useMemo(() => data ? subjects.map(subject => ({ ...subjectLane(subject, data.levels), progress: exactLanes.get(subject.countryId)?.progress ?? [] })) : [], [data, subjects, exactLanes]);
  const numericLaneAt = useCallback((id: string, hour: number) => {
    const lane = exactLanes.get(id);
    return lane && data && !data.exactHours.includes(Math.floor(hour)) && inSpans(data.summaries[id]?.approximate ?? NO_SPANS, hour)
      ? { ...lane, runs: [] } : lane;
  }, [data, exactLanes]);
  useParityRows(root, () => ({ kind: 'chart', label: 'Autonomy scores and thresholds', scope: 'displayed-hour',
    complete: !!data?.exactHours.includes(head) && !!identities, total: lanesData.length,
    rows: lanesData.map(lane => ({ countryId: lane.id, run: numericLaneAt(lane.id, head) ? runOf(numericLaneAt(lane.id, head)!, head) : null })),
    levels: data?.levels, recorded: data?.recorded, unidentified: data?.unidentified, hour: head }));
  // Level colours in level order (the palette's fixed order), never by a level's name.
  const palette = useMemo(() => categoricalPalette(tokens.surface), [tokens.surface]);
  const levelColour = (level: number) => palette[level % palette.length];

  // The lanes' names and flags as recorded near the moment shown, never across a gap
  // (`useCountryLooks`), and a tooltip's countries as recorded at the hour hovered
  // (`useHoverLabels`), each part (name, flag) on its own.
  const laneTags = useMemo(() => lanesData.map(lane => identities?.aliasesAt(new Set([lane.id]), head)[0] ?? null), [lanesData, identities, head]);
  const segmentAt = useCallback((tag: string, hour: number) => runAt(identityHistories.get(tag) ?? [], hour)?.firstHour ?? null, [identityHistories]);
  // Names for every lane (the label and order use them); flags only for the lanes in view.
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const onLanesInView = useCallback((first: number, last: number) => setInView([first, last]), []);
  const flagTags = useMemo(() => laneRequests(laneTags, inView).looks.filter((tag): tag is string => tag !== null), [laneTags, inView]);
  const looks = useCountryLooks(runtime.client, track, laneTags.filter((tag): tag is string => tag !== null), head, active, segmentAt, undefined, false, flagTags);
  const metadataNames = useMemo(() => new Map(track?.source.metadata.countries.map(country => [country.tag, country.name])), [track?.source]);
  const metadataName = (tag: string) => metadataNames.get(tag) ?? 'Country name unavailable';
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const hover = useHoverLabels(runtime.client, track, active, useMemo(() => (tag: string) => ({ name: metadataName(tag) }), [metadataNames]));

  const menu = useContextMenu('Timeline actions');
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(missingNativeSubjects ? [`Autonomy histories with unrecorded native subject identity spans: ${missingNativeSubjects}.`] : []),
      ...(missingNativeOverlords ? [`Autonomy histories with unrecorded native overlord identity spans affecting selection: ${missingNativeOverlords}.`] : []),
      ...(groups.error ? [`Country identity changes could not be loaded: ${groups.error}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, missingNativeSubjects, missingNativeOverlords, subjects.length, lanesData.length, onDiagnostics]);

  const lanes = useMemo<TimelineLane[]>(() => !data ? [] : lanesData.map((lane, index) => {
    const exact = numericLaneAt(lane.id, head) ?? { ...lane, runs: [] };
    const run = runOf(exact, head);
    const tag = laneTags[index];
    const labels = !data.exactHours.includes(head) && !run ? { label: 'Loading autonomy…', pill: 'Loading…', description: 'Loading autonomy…' }
      : laneLabels(data, exact, head, unobserved, tag !== null);
    const colourOf = (span: string) => levelColour(lane.spanLevels.get(span) ?? 0);
    return {
      id: lane.id, label: labels.label, spans: lane.spans,
      identity: { label: tag === null ? lane.tag : looks.name(tag), countryFlag: tag === null
        ? { flagReason: 'Country unavailable at this time.' } : { flag: looks.flag(tag),
          flagPending: looks.flagPending?.(tag), flagReason: looks.flagReason?.(tag) } },
      pill: { text: labels.pill, color: run ? levelColour(run.level) : mix(tokens.surface, tokens.text, 0.25) },
      // Not a subject at the moment shown (released, not yet a subject, or not recorded): faded.
      muted: !run || run.overlord == null,
      paint: (ctx, geometry, chartTokens) => paintLane(ctx, lane, geometry, chartTokens, colourOf,
        prepared.geometryReady ? data.summaries[lane.id] : undefined, approximate),
    };
  }), [data, prepared.geometryReady, approximate, lanesData, numericLaneAt, laneTags, head, looks, palette, tokens.surface, tokens.text, unobserved]); // eslint-disable-line react-hooks/exhaustive-deps

  const groupFailure = groups.error && (widget.countries.kind === 'group' || !identityRows)
    ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  const content = (() => {
  if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see its subjects’ autonomy." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (groupFailure && !groups.data) return groupFailure;
  const reason = hourlyGroup ? selection.reason : filter.reason;
  if (widget.countries.kind === 'group' && !groups.data) return <WidgetState kind="loading" label="Loading country membership…" />;
  if (reason) return <WidgetState kind="empty" message={reason} />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data || !identities) return <WidgetState kind="loading" label="Loading autonomy…" />;
  if (!lanesData.length && (!prepared.geometryReady || autonomyUnprepared(data, { firstHour: first, lastHour: last }))) return <>
    <WidgetState kind="loading" label="Subject data is still being prepared." />
    <PreparedKey approximate={approximate} pending={pending} />
  </>;
  // Tables missing at every hour: not recorded, never "no subjects".
  if (!data.recorded.length) return <WidgetState kind="unavailable" message="Subjects’ autonomy was not recorded." />;
  if (!lanesData.length) {
    // Countries chosen at hours still being prepared are not known yet: no lane is not proof of none.
    if (!(hourlyGroup ? selection.known : filter.known)) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
    if (missingNativeSubjects) return <WidgetState kind="unavailable" message="Subject data is unavailable for these countries." />;
    if (missingNativeOverlords) return <WidgetState kind="unavailable" message="Subject data is unavailable for these countries." />;
    // Unknown membership that cut a lane away is said, never read as "no subjects".
    return <WidgetState kind="empty" message={noLanesMessage(data, { firstHour: first, lastHour: last },
      membershipUnknownFor(chosen, bySubject, unobserved))} />;
  }

  const description = `${dayLabel(head)}: ${lanesData.map((lane, index) => {
    const tag = laneTags[index];
    return `${tag === null ? lane.tag : looks.name(tag)}: ${!data.exactHours.includes(head) && !runOf(numericLaneAt(lane.id, head) ?? { ...lane, runs: [] }, head) ? 'Loading autonomy…' : laneLabels(data, numericLaneAt(lane.id, head) ?? { ...lane, runs: [] }, head, unobserved, tag !== null).description}`;
  }).join('; ')}.`;
  const binding = (id: string) => ({ id,spans: subjects.find(subject => subject.countryId === id)?.runs.flatMap(run => identities.aliasesAt(new Set([id]),run.firstHour)
    .map(tag => ({tag,firstHour:run.firstHour,lastHour:run.lastHour,startsSegment:run.startsSegment}))) ?? [] });
  return <div className="autonomy">
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label={`Autonomy of ${laneTags.map((tag, index) => tag === null ? lanesData[index].tag : looks.name(tag)).join(', ')}`} extent={extent} playhead={playhead} labelHour={head} preserveZoomOnExtentChange={options.grow === true} lanes={lanes}
      rowHeight={LANE_HEIGHT} showIcons={false} showNames={false} description={description} hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
      active={active} revision={`${head}:${data.branchId}:${looks.revision}:${hover.revision}`} approximate={approximate} pending={pending}
      onHover={setHoverHour} onLanesInView={onLanesInView}
      onSummaryView={(view, visible) => prepared.onSummaryView(view,visible.map(lane => binding(lane.id)))}
      onNavigateHour={(lane, from, forward) => prepared.navigate(lane.id,from,forward,binding(lane.id))}
      tooltip={(lane, _span, hour) => {
        const subject = numericLaneAt(lane.id, hour);
        const run = subject ? runOf(subject, hour) : null;
        // The subject and its overlord are named as recorded at the hour hovered.
        const tag = identities.aliasesAt(new Set([lane.id]), hour)[0] ?? null;
        const overlordTag = run?.overlord == null || identities.keyAt(run.overlord, hour) === null ? null : run.overlord;
        hover.ask(hour, [...(tag === null ? [] : [tag]), ...(overlordTag === null ? [] : [overlordTag])]);
        const named = hover.labelAt(hour);
        const label = tag === null ? { name: 'Country name unavailable', flagReason: 'Country unavailable at this time.' } : named(tag);
        const name = label.name;
        if (!data.exactHours.includes(Math.floor(hour)) && !run) return { title: `${name} · ${dayLabel(hour)}`, countryFlag: label, rows: [{ label: 'Autonomy', value: 'Loading…', tone: 'muted' }] };
        if (!subject || !run) {
          return { title: `${name} · ${dayLabel(hour)}`, countryFlag: label, rows: [{ label: 'Autonomy', value: tag === null ? 'Country unavailable at this time.' : absentAt(data, hour, unobserved).text, tone: 'muted' }] };
        }
        const overlord = overlordTag == null ? null : named(overlordTag);
        const level = levelName(data.levels[run.level]);
        const rows: TooltipRow[] = [
          { label: 'Subject level', value: level, swatch: levelColour(run.level) },
          // An hour with an overlord the recording does not name may have hidden this one.
          { label: 'Overlord', value: overlord?.name ?? (run.overlord !== null ? 'Country unavailable at this time.' : inSpans(data.unidentified, hour) ? 'Not known at this hour' : 'None recorded'), countryFlag: overlord ?? undefined, tone: overlord ? undefined : 'muted' },
          { label: run.highest ? 'Progress within the level' : 'Progress to the next level', value: formatProgress(run) },
          { label: 'Autonomy score', value: formatScore(data, run.scoreRaw), divider: true },
          { label: 'Moves down below', value: `${formatScore(data, run.lowerRaw)}${run.lowest ? ' · no lower level' : ''}` },
          { label: 'Moves up at', value: `${formatScore(data, run.upperRaw)}${run.highest ? ' · no higher level' : ''}` },
        ];
        return { title: `${name} · ${dayLabel(hour)}`, countryFlag: label, rows,
          note: contentTitleHint(data.levels[run.level]?.id) };
      }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    <PreparedKey approximate={approximate} pending={pending} />
    {menu.menu}
  </div>;
  })();
  return <div ref={root} style={{ display: 'contents' }}>{groups.data && groupFailure}{content}</div>;
}
