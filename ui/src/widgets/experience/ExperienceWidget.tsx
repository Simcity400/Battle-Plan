import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import LineSwatch from '../../charts/LineSwatch';
import { lineDashes } from '../../charts/palette';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer } from '../../charts/TimeChart';
import { drawLineSummary, summaryExtent } from '../../charts/chartSummary';
import type { ChartSummary } from '../../charts/chartSummary';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { motionOf } from '../../charts/easing.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { countryLineColours } from '../../dashboard/countryLineColours';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveCountries } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { trackHourOf } from '../../dashboard/settledHour';
import { useTiles } from '../../dashboard/tiles/useTiles';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { followStore } from '../../dashboard/follow/store.ts';
import { followRefusal, followsHead, useFollow } from '../../dashboard/follow/useFollow.ts';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { definition, OPTIONS } from './definition';
import { BRANCHES, BRANCH_LABELS, MEASURE_LABELS, SCALE, experienceBranches, experienceCard, experienceFamily, experienceLines, experienceMeasure, followExperienceHeads,
  experienceStructure, formatExperience, pointsSummary, validateExperienceReply } from './experienceData';
import type { ExperienceBranch } from './experienceData';
import { summaryEnd } from '../country-measure/measureData';
import './experience.css';
import { SPEC, at as pickAt, recordedHour } from '../../dashboard/regionTime.ts';

const NO_WARNINGS: string[] = [];
const NO_PAINT = () => {};
const branchDash = (branch: ExperienceBranch) => lineDashes(BRANCHES.indexOf(branch)).primary;

export default function ExperienceWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = experienceMeasure(options), branches = experienceBranches(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const grouped = widget.countries.kind === 'group';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryKey(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);
  const following = widget.time.mode === 'follow';
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const gate = useGate(runtime, widget, SPEC, active);
  const lane = (branch: ExperienceBranch) => track && resolved.tags.length && branches.includes(branch) ? { source: track.source, span: gate.span, branchId: track.branch.id,
    family: experienceFamily(measure, branch), tags: resolved.tags, ...(resolved.membership ? { within: resolved.membership } : {}), lastHour } : null;
  const tiles = { army: useTiles(runtime.client, lane('army'), active, following), navy: useTiles(runtime.client, lane('navy'), active, following),
    air: useTiles(runtime.client, lane('air'), active, following) };
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const headQuery = (branch: ExperienceBranch) => following && active && track && resolved.tags.length && branches.includes(branch)
    ? gate.follow({ product: 'series-state', params: { branchId: track.branch.id, family: experienceFamily(measure, branch), reads: resolved.tags.map(tag => ({ country: countryFromKey(tag) })) } }) : null;
  const followed = {
    army: useFollow(store, useMemo(() => headQuery('army'), [following, active, track?.branch.id, measure, branches.join(), resolved.tags, gate.follow]), root, NO_PAINT),
    navy: useFollow(store, useMemo(() => headQuery('navy'), [following, active, track?.branch.id, measure, branches.join(), resolved.tags, gate.follow]), root, NO_PAINT),
    air: useFollow(store, useMemo(() => headQuery('air'), [following, active, track?.branch.id, measure, branches.join(), resolved.tags, gate.follow]), root, NO_PAINT),
  };
  const entryOf = (branch: ExperienceBranch) => followsHead(followed[branch]) ? followed[branch].entry : null;
  const refused = followRefusal(followed.army, followed.navy, followed.air);
  const heads = { army: entryOf('army'), navy: entryOf('navy'), air: entryOf('air') };
  useEffect(() => followExperienceHeads(heads, resolved.tags, (branch, tag) => tiles[branch].head(tag)),
    [heads.army, heads.navy, heads.air, resolved.tags, tiles.army.head, tiles.navy.head, tiles.air.head]);
  const first = resolved.tags[0] ?? null;
  const [hover, setHover] = useState<number | null>(null);
  const card = useHeadProduct(runtime, useGate(runtime, widget, hover === null ? SPEC : pickAt(hover)), 'country-experience', track && first ? { country: countryFromKey(first)} : null,
    { active, validate: validateExperienceReply, structure: experienceStructure });
  const setRoots = useCallback((element: HTMLDivElement | null) => { setRoot(element); card.root(element); }, [card.root]);
  const styleAt = Math.max(extent[0], Math.min(moment, extent[1]));
  const styles = useCountryLineStyles(runtime.client, gate, track, resolved.tags, styleAt, active);
  const labels = useCountryLabelsAt(runtime.client, track, useMemo(() => resolved.tags.map(tag => ({ tag, hour: styleAt })), [resolved.tags, styleAt]), active);
  const single = resolved.tags.length === 1;
  const lines = useMemo(() => experienceLines(resolved.tags, branches, single,
    countryLineColours(resolved.tags, styles.known, tokens.surface, tokens.countryColours === 'fixed'), tokens.categories),
  [resolved.tags, branches.join(), single, styles.known, tokens]);
  const warnings = card.data?.warnings;
  useLayoutEffect(() => {
    onDiagnostics(warnings?.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);
  const summaryOf = useMemo(() => {
    const scaled = new WeakMap<ChartSummary, ChartSummary>();
    return (branch: ExperienceBranch, tag: string) => {
      const summary = tiles[branch].read()?.get(tag)?.summary;
      if (!summary) return undefined;
      let made = scaled.get(summary);
      if (!made) scaled.set(summary, made = pointsSummary(summary));
      return made;
    };
  }, [tiles.army.read, tiles.navy.read, tiles.air.read]);
  const until = shownUntil(playhead, options.playback as PlaybackMode, options.afterPlayhead === true);
  const layers = useMemo<ChartLayer[]>(() => lines.map(line => ({
    draw: (ctx, frame, chartTokens, state) => {
      const summary = summaryOf(line.branch, line.tag);
      if (summary) drawLineSummary(ctx, frame, chartTokens, summary, line.color, false, state.playhead, { dash: single ? lineDashes(0).primary : branchDash(line.branch) });
    },
    extent: x => { const summary = summaryOf(line.branch, line.tag); return summary ? summaryExtent(summary, x, until) : null; },
  })), [lines, summaryOf, until, single]);
  const data = useMemo<ChartData>(() => ({ revision: () => BRANCHES.map(branch => tiles[branch].read()),
    subscribe: listener => { const stops = BRANCHES.map(branch => tiles[branch].subscribe(listener)); return () => stops.forEach(stop => stop()); } }),
  [tiles.army.subscribe, tiles.navy.subscribe, tiles.air.subscribe]);
  const menu = useContextMenu('Experience actions');
  const head = card.data ? card.data : null;
  const error = BRANCHES.map(branch => tiles[branch].error).find(Boolean);
  if (!track) return <WidgetState kind="empty" icon="sparkle" message="Load a campaign to see experience." />;
  if (refused) return <WidgetState kind="unavailable" technical={refused} />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (grouped && groups.error && !groups.data) return <WidgetState kind="error" what="country groups" failure={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" icon="sparkle" message={resolved.reason} />;
  if (grouped && !groups.data) return <WidgetState kind="loading" label="Loading experience…" />;
  if (!branches.length) return <WidgetState kind="empty" message="Turn on an experience branch in the widget settings." />;
  const legend = single ? branches.map(branch => {
    const row = head ? experienceCard(head, branch).rows.find(item => item.label === MEASURE_LABELS[measure]) : null;
    return { id: branch, label: BRANCH_LABELS[branch], color: lines.find(line => line.branch === branch)?.color, value: row ? String(row.value) : card.outside ? 'Outside the recording' : 'Loading…' };
  }) : resolved.tags.map(tag => ({ id: tag, label: labels.labelAt(styleAt)(tag).name, color: lines.find(line => line.tag === tag)?.color }));
  const title = `Experience, ${MEASURE_LABELS[measure].toLowerCase()}`;
  return <div className="experience" ref={setRoots}>
    {error ? <WidgetState kind="error" failure={error as Error} onRetry={() => BRANCHES.forEach(branch => tiles[branch].retry())} /> : null}
    <Legend label={single ? 'Branches' : 'Countries'} items={legend} />
    {!single && <p className="chart-line-key">{branches.map(branch => <span key={branch}><LineSwatch dash={branchDash(branch)} />{BRANCH_LABELS[branch]}</span>)}</p>}
    <div className="experience-chart">
      <TimeChart label={`${title} of ${resolved.tags.map(tag => labels.labelAt(styleAt)(tag).name).join(', ')}`} extent={extent} playhead={playhead}
        playback={options.playback as PlaybackMode} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour} liveHead={liveHead}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        padding={{ right: options.endLabels ? CHART_LAYOUT.measureEndLabels : CHART_LAYOUT.right }} layers={layers} data={data} active={active}
        notRecorded={branches.every(branch => tiles[branch].unrecorded) ? tiles[branches[0]].unrecorded : null}
        revision={`${track.branch.id}:${lines.length}:${measure}`}
        onSummaryView={view => BRANCHES.forEach(branch => tiles[branch].onView(view))}
        onHover={hour => setHover(hour === null ? null : Math.floor(hour))}
        formatValue={value => formatExperience(value * SCALE, measure !== 'banked')}
        tooltip={hour => ({ title: dayLabel(hour), subtitle: first ? `${labels.labelAt(Math.floor(hour))(first).name}: ${title}` : title,
          rows: recordedHour(track, hour) === null ? [{ label: 'Outside the recording', tone: 'muted' as const }]
            : head && card.at === Math.floor(hour) ? branches.flatMap(branch => experienceCard(head, branch).rows.filter(row => row.label === MEASURE_LABELS[measure])
            .map(row => ({ ...row, label: BRANCH_LABELS[branch] }))) : [{ label: 'Loading…', tone: 'muted' as const }] })}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          placeEndLabels(ctx, frame, chartTokens, lines.flatMap(line => {
            const value = summaryEnd(summaryOf(line.branch, line.tag), at);
            if (value === null) return [];
            return [{ id: `${line.tag}|${line.branch}`, y: frame.toY(value), color: line.color,
              text: single ? BRANCH_LABELS[line.branch] : `${labels.labelAt(styleAt)(line.tag).name} · ${BRANCH_LABELS[line.branch]}`,
              suffix: formatExperience(value * SCALE, measure !== 'banked') }];
          }));
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {menu.menu}
  </div>;
}
