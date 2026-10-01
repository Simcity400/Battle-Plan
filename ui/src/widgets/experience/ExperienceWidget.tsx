import { useEffect, useMemo, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import type { EventBadge } from '../../charts/badges';
import { drawLine, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import LineSwatch from '../../charts/LineSwatch';
import { lineDashes, lineStyles } from '../../charts/palette';
import { clipToSpans, extentOf, lineEndHour, runAt } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryLineColours, uncolouredWarning } from '../../dashboard/countryLineColours';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { membershipChanges, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles';
import type { ExperienceReply } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition';
import { BRANCHES, BRANCH_LABELS, experienceRaw, experienceRuns, experienceTooltip, formatExperience, validateExperienceReply } from './experienceData';
import type { ExperienceMeasure } from './experienceData';
import './experience.css';

const NO_WARNINGS: string[] = [];
const END_LABEL_ROOM = 128;
/** Comparison labels also name the branch. */
const COMPARE_LABEL_ROOM = 184;
/** With several countries, colour names the country and the dash the branch. */
const branchDash = (branch: typeof BRANCHES[number]) => lineDashes(BRANCHES.indexOf(branch)).primary;

/** Banked experience or net change per day, with each branch independently shown. */
export default function ExperienceWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = options.measure as ExperienceMeasure;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const fallback = defaultCountryTag(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-experience',
    params: { branchId: track.branch.id, tags: resolved.tags } } : null;
  const { data, error, retry } = useProduct<ExperienceReply>(runtime.client, request, active, validateExperienceReply);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  const styles = useCountryLineStyles(runtime.client, track, resolved.tags, head, active);
  const playback = options.playback as PlaybackMode;
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const branches = BRANCHES.filter(branch => options[branch] === true);
  const single = resolved.tags.length === 1;
  const branchStyles = lineStyles(BRANCHES.map((key, preferred) => ({ key, preferred })), tokens.categories);
  const [tipHour, setTipHour] = useState<number | null>(null);
  const wanted = useMemo(() => tipHour == null ? [] : resolved.tags.map(tag => ({ tag, hour: tipHour })), [resolved.tags, tipHour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  useEffect(() => { if (!active) setTipHour(null); }, [active]);
  const menu = useContextMenu('Experience actions');
  useEffect(() => {
    const uncoloured = uncolouredWarning((data?.series ?? []).map(item => item.tag), styles.known, !!styles.data);
    const warnings = [...(data?.warnings ?? []), ...(styles.data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(uncoloured ? [uncoloured] : []),
      ...(resolved.dropped ? [`${resolved.dropped} more countries are left out; this widget draws at most ${resolved.tags.length}.`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, styles.data, styles.known, groups.data, resolved.dropped, resolved.tags.length, onDiagnostics]);

  const countries = useMemo(() => {
    if (!data) return [];
    const colours = countryLineColours(data.series.map(item => item.tag), styles.known, tokens.surface, tokens.countryColours === 'fixed');
    return data.series.map(item => {
      const membership = resolved.membership?.get(item.tag);
      const inRange = clipToSpans(item.reads, [{ firstHour: extent[0], lastHour: extent[1] }]);
      return { tag: item.tag, reads: membership ? clipToSpans(inRange, membership) : inRange,
        readHours: item.reads.map(read => read.firstHour), recorded: item.reads.length > 0, membership,
        name: styles.known.get(item.tag)?.name ?? track?.source.metadata.countries.find(country => country.tag === item.tag)?.name ?? item.tag,
        color: colours.get(item.tag)!,
        colours: Object.fromEntries(BRANCHES.map(branch => [branch, single ? branchStyles.get(branch)!.color : colours.get(item.tag)!])) as Record<typeof BRANCHES[number], string> };
    });
  }, [data, styles.known, tokens, resolved.membership, extent[0], extent[1], track, single]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => data ? countries.flatMap(country => BRANCHES.filter(branch => options[branch] === true).map(branch => ({
    id: JSON.stringify([country.tag, branch]), country, branch, color: country.colours[branch],
    dash: single ? lineDashes(0).primary : branchDash(branch),
    runs: experienceRuns(country.reads, data.scale, branch, measure),
  }))) : [], [data, countries, single, options.army, options.navy, options.air, measure]);
  const layers = useMemo<ChartLayer[]>(() => lines.map<ChartLayer>(line => ({
    draw: (ctx, frame, chartTokens, state) => drawLine(ctx, frame, chartTokens, line.runs, line.color, { dash: line.dash, until: state.playhead ?? undefined }),
    extent: x => extentOf(line.runs, x[0], Math.min(x[1], until ?? x[1])),
  })), [lines, until]);
  const keyHours = useMemo(() => [...new Set(countries.flatMap(country => country.readHours))].sort((a, b) => a - b), [countries]);
  const badges = useMemo<EventBadge[]>(() => countries.flatMap(country => country.membership
    ? membershipChanges(country.membership, extent[0], extent[1], resolved.unobserved).map(change => ({
      hour: change.hour, glyph: change.change === 'joined' ? '+' : '−', label: `${country.name} ${change.change}`, color: country.color })) : []),
  [countries, extent[0], extent[1], resolved.unobserved]);

  if (!track) return <WidgetState kind="empty" icon="sparkle" message="Load a campaign to see experience." />;
  if ((widget.time.mode === 'pin' || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (widget.countries.kind === 'group' && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label="Loading experience…" />;
  if (!countries.some(country => country.recorded)) return <WidgetState kind="unavailable" message="Not in this recording" />;
  if (!branches.length) return <WidgetState kind="empty" message="Turn on an experience branch in the widget settings." />;

  return <div className="experience">
    <Legend label={single ? 'Branches' : 'Countries'} items={single ? lines.map(line => {
      const read = runAt(line.country.reads, head);
      const raw = read ? experienceRaw(read, line.branch, measure) : null;
      return { id: line.id, label: BRANCH_LABELS[line.branch], color: line.color, value: raw == null ? 'Not known' : formatExperience(raw, data.scale, measure) };
    }) : countries.map(country => ({ id: country.tag, label: country.name, color: country.color,
      value: country.recorded ? undefined : 'Not in this recording' }))} />
    {!single && <p className="chart-line-key">{branches.map(branch => <span key={branch}>
      <LineSwatch dash={branchDash(branch)} />{BRANCH_LABELS[branch]}
    </span>)}</p>}
    <div className="experience-chart">
      <TimeChart label={`Experience of ${countries.map(country => country.name).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }} fitValues={shown => shown}
        padding={{ right: options.endLabels ? (single ? END_LABEL_ROOM : COMPARE_LABEL_ROOM) : 14 }} layers={layers} keyHours={keyHours} badges={badges}
        active={active} revision={`${head}:${data.branchId}:${lines.length}:${viewport}:${labels.revision}`}
        tooltip={(hour, _frame, hovered) => {
          if (hour < extent[0] || hour > extent[1]) return null;
          setTipHour(hour);
          return experienceTooltip(data, countries, branches, measure, hour, labels.labelAt(hour), viewport, badgeRows(hovered));
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          const ends = lines.flatMap(line => {
            const last = lineEndHour(line.runs, frame.x[0], at);
            const read = last == null ? null : runAt(line.country.reads, last);
            const raw = read ? experienceRaw(read, line.branch, measure) : null;
            if (raw == null) return [];
            return [{ id: line.id, y: frame.toY(Number(raw) / data.scale), color: line.color,
              text: `${single ? BRANCH_LABELS[line.branch] : `${line.country.name} · ${BRANCH_LABELS[line.branch]}`} ${formatExperience(raw, data.scale, measure)}` }];
          });
          placeEndLabels(ctx, frame, chartTokens, ends);
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {menu.menu}
  </div>;
}
