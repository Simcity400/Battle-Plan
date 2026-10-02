import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel, tracePath } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { categoricalPalette, SECONDARY_DASH } from '../../charts/palette';
import { curveSegments } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import type { LaneGeometry, TimelineLane } from '../../charts/timeline/Timeline';
import type { ChartTokens } from '../../charts/tokens';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupRuns, groupUnobserved, worldFilter } from '../../dashboard/groups';
import { Lineage, countryMembership } from '../../dashboard/identity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { inSpans } from '../../charts/timeline/layout';
import type { AutonomyReply } from '../../generated/dashboard';
import { mix } from '../../shell/appearance/color';
import { absentAt, autonomyViews, chosenViews, clipSubject, formatProgress, formatScore, laneLabels, levelName, membershipUnknownFor, noLanesMessage, runOf, subjectLane,
  validateAutonomyReply } from './autonomyData';
import type { SubjectLane } from './autonomyData';
import { OPTIONS } from './definition';
import './autonomy.css';

const NO_WARNINGS: string[] = [];
const NO_SPANS: { firstHour: number; lastHour: number }[] = [];
/** Most subjects drawn at once; the rest are named in the diagnostics. */
const MAX_LANES = 64;
/** A lane's height, as signed off (mock-up diplo-1). */
const LANE_HEIGHT = 54;
/** Room between a lane's edge and its thresholds. */
const LANE_PAD = 8;

/** One subject's lane (mock-up diplo-1): its level over time as coloured bands,
 * the move-up and move-down scores as the dashed top and bottom edges, its
 * progress between them as a line, and a dot where the level changed. */
function paintLane(ctx: CanvasRenderingContext2D, lane: SubjectLane, geometry: LaneGeometry, tokens: ChartTokens, colour: (span: string) => string): void {
  const top = geometry.top + LANE_PAD;
  const height = geometry.height - 2 * LANE_PAD;
  const bottom = top + height;
  const end = geometry.playhead == null ? Infinity : geometry.playhead;
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
  const toY = (value: number) => bottom - Math.max(0, Math.min(1, value)) * height;
  const last = Math.min(geometry.hours[1], end);
  ctx.save();
  ctx.strokeStyle = tokens.text; ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round'; ctx.lineCap = 'round';
  for (const { points, path } of curveSegments(lane.progress, geometry.hours[0], last, geometry.toX, toY)) {
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
}

/** Each subject's autonomy over time (mock-up diplo-1): one lane per subject with
 * its flag and name, its current level as a coloured pill underneath, and the
 * playhead through every lane. Level names and order come from the recording's
 * content (its autonomy states); level colours are the palette in that order. */
export default function AutonomyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryTag(track);
  const lineage = useMemo(() => new Lineage(groups.data?.identityChanges ?? []), [groups.data?.identityChanges]);
  const bySubject = options.countriesAre === 'subjects';
  // Read the branch once; country identities and group membership are resolved locally.
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'autonomy',
    params: { branchId: track.branch.id, overlords: [], subjects: [] } } : null;
  const { data, error, retry } = useProduct<AutonomyReply>(runtime.client, request, active, validateAutonomyReply);

  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  const filter = useMemo(() => worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, head, lastHour, lineage),
    [widget.countries, fallback, groups.data, head, lastHour, lineage]);
  // Hourly groups choose every member's history, then clip it at membership hours.
  const selection = useMemo(() => worldFilter(widget.countries.kind === 'group' && widget.countries.membership === 'hour'
    ? { ...widget.countries, membership: 'ever' } : widget.countries, fallback ? [fallback] : [], groups.data, head, lastHour, lineage),
  [widget.countries, fallback, groups.data, head, lastHour, lineage]);
  const hourlyGroup = widget.countries.kind === 'group' && widget.countries.membership === 'hour';
  const membership = useMemo(() => hourlyGroup && widget.countries.kind === 'group' && groups.data
    ? countryMembership(groupRuns(groups.data, widget.countries.group) ?? [], lineage) : null, [hourlyGroup, widget.countries, groups.data, lineage]);

  // A group counted at each hour shows each subject only while it (or its overlord) belongs,
  // and hours whose membership was not recorded as unknown. A group counted at one moment
  // chose its countries there: its unrecorded hours elsewhere hide nothing.
  const unobserved = useMemo(() => hourlyGroup && widget.countries.kind === 'group' && groups.data
    ? groupUnobserved(groups.data, widget.countries.group) : NO_SPANS, [hourlyGroup, widget.countries, groups.data]);
  const views = useMemo(() => data ? autonomyViews(data, lineage) : [], [data, lineage]);
  const chosen = useMemo(() => selection.countryIds ? chosenViews(views, selection.countryIds, bySubject, lineage) : views,
    [views, selection.countryIds, bySubject, lineage]);
  const subjects = useMemo(() => chosen.map(subject => clipSubject(subject, bySubject, membership, unobserved, lineage)).filter(subject => subject.runs.length),
    [chosen, bySubject, membership, unobserved, lineage]);
  const lanesData = useMemo(() => data ? subjects.slice(0, MAX_LANES).map(subject => subjectLane(subject, data.levels)) : [], [data, subjects]);
  // Level colours in level order (the palette's fixed order), never by a level's name.
  const palette = useMemo(() => categoricalPalette(tokens.surface), [tokens.surface]);
  const levelColour = (level: number) => palette[level % palette.length];

  // The lanes' names and flags as recorded near the moment shown, never across a gap
  // (`useCountryLooks`), and a tooltip's countries as recorded at the hour hovered
  // (`useHoverLabels`), each part (name, flag) on its own.
  const laneTags = useMemo(() => lanesData.map(lane => lineage.tagOf(lane.id, head)), [lanesData, lineage, head]);
  const looks = useCountryLooks(runtime.client, track, laneTags, head, active);
  const metadataNames = useMemo(() => new Map(track?.source.metadata.countries.map(country => [country.tag, country.name])), [track?.source]);
  const metadataName = (tag: string) => metadataNames.get(tag) ?? tag;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const hover = useHoverLabels(runtime.client, track, active, useMemo(() => (tag: string) => ({ name: metadataName(tag) }), [metadataNames]));

  const menu = useContextMenu('Timeline actions');
  useEffect(() => {
    const dropped = subjects.length - lanesData.length;
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(dropped > 0 ? [`${dropped} more subjects are left out; this widget draws at most ${MAX_LANES}.`] : []),
      ...(groups.error ? [`Country identity changes could not be loaded: ${groups.error}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, subjects.length, lanesData.length, onDiagnostics]);

  const lanes = useMemo<TimelineLane[]>(() => !data ? [] : lanesData.map((lane, index) => {
    const run = runOf(lane, head);
    const labels = laneLabels(data, lane, head, unobserved);
    const colourOf = (span: string) => levelColour(lane.spanLevels.get(span) ?? 0);
    return {
      id: lane.id, label: labels.label, spans: lane.spans,
      identity: { label: looks.name(laneTags[index]), icon: looks.flag(laneTags[index]) },
      pill: { text: labels.pill, color: run ? levelColour(run.level) : mix(tokens.surface, tokens.text, 0.25) },
      // Not a subject at the moment shown (released, not yet a subject, or not recorded): faded.
      muted: !run || run.overlord == null,
      paint: (ctx, geometry, chartTokens) => paintLane(ctx, lane, geometry, chartTokens, colourOf),
    };
  }), [data, lanesData, laneTags, head, looks, palette, tokens.surface, tokens.text, unobserved]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see its subjects’ autonomy." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (widget.countries.kind === 'group' && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  const reason = hourlyGroup ? selection.reason : filter.reason;
  if (reason) return <WidgetState kind="empty" message={reason} />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label="Loading autonomy…" />;
  // Tables missing at every hour: not recorded, never "no subjects".
  if (!data.recorded.length) return <WidgetState kind="unavailable" message="Subjects’ autonomy was not recorded in this campaign branch." />;
  if (!lanesData.length) {
    // Unknown membership that cut a lane away is said, never read as "no subjects".
    return <WidgetState kind="empty" message={noLanesMessage(data, { firstHour: track.branch.firstHour, lastHour: track.branch.lastHour }, bySubject,
      [...selection.tags ?? []].map(metadataName), membershipUnknownFor(chosen, bySubject, unobserved))} />;
  }

  const byId = new Map(lanesData.map(lane => [lane.id, lane]));
  const description = `${dayLabel(head)}: ${lanesData.map((lane, index) => {
    return `${looks.name(laneTags[index])}: ${laneLabels(data, lane, head, unobserved).description}`;
  }).join('; ')}.`;
  return <div className="autonomy">
    <Timeline label={`Autonomy of ${laneTags.map(tag => looks.name(tag)).join(', ')}`} extent={extent} playhead={playhead} lanes={lanes}
      rowHeight={LANE_HEIGHT} showIcons={false} showNames={false} description={description} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
      active={active} revision={`${head}:${data.branchId}:${looks.revision}:${hover.revision}`}
      tooltip={(lane, _span, hour) => {
        const subject = byId.get(lane.id);
        const run = subject ? runOf(subject, hour) : null;
        // The subject and its overlord are named as recorded at the hour hovered.
        const tag = lineage.tagOf(lane.id, hour);
        const overlordTag = run?.overlord == null ? null : lineage.tagOf(lineage.countryAt(run.overlord, run.firstHour), hour);
        hover.ask(hour, overlordTag != null ? [tag, overlordTag] : [tag]);
        const named = hover.labelAt(hour);
        const name = named(tag).name;
        if (!subject || !run) {
          return { title: `${name} · ${dayLabel(hour)}`, rows: [{ label: 'Autonomy', value: absentAt(data, hour, unobserved).text, tone: 'muted' }] };
        }
        const overlord = overlordTag == null ? null : named(overlordTag);
        const level = levelName(data.levels[run.level]);
        const rows: TooltipRow[] = [
          { label: 'Subject level', value: level, swatch: levelColour(run.level) },
          // An hour with an overlord the recording does not name may have hidden this one.
          { label: 'Overlord', value: overlord?.name ?? (inSpans(data.unidentified, hour) ? 'Not known at this hour' : 'None recorded'), icon: overlord?.flag, tone: overlord ? undefined : 'muted' },
          { label: run.highest ? 'Progress within the level' : 'Progress to the next level', value: formatProgress(run) },
          { label: 'Autonomy score', value: formatScore(data, run.scoreRaw), divider: true },
          { label: 'Moves down below', value: `${formatScore(data, run.lowerRaw)}${run.lowest ? ' · no lower level' : ''}` },
          { label: 'Moves up at', value: `${formatScore(data, run.upperRaw)}${run.highest ? ' · no higher level' : ''}` },
        ];
        return { title: `${name} · ${dayLabel(hour)}`, rows };
      }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
