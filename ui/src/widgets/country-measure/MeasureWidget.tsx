import { useEffect, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { SourceFlag } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import type { EventBadge } from '../../charts/badges';
import { dayLabel, drawEventTicks, drawLine, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { SECONDARY_DASH } from '../../charts/palette';
import { extentOf, lineEndHour } from '../../charts/series';
import { formatFixed } from '../../charts/valueAxis';
import type { Run } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { countryLineColours, uncolouredWarning } from '../../dashboard/countryLineColours';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles';
import type { WidgetProps } from '../../dashboard/definitions';
import { clipRuns, isMember, membershipChanges, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { readHourAt, validateStabilityReply } from '../../dashboard/stabilityReply';
import { formatDateTime } from '../../dashboard/calendar';
import type { OptionField } from '../../dashboard/options';
import type { CountryStabilityReply, Measure, MeasureReply } from '../../generated/dashboard';
import { formatBalance, nearestDrop, netSpend, rawAt, toRuns, validateMeasureReply } from './measureData';
import { MEASURE_OPTIONS } from './options';
import './measure.css';

const NO_WARNINGS: string[] = [];
/** Hovering within this many pixels of a spend tick snaps to it. */
const SNAP_PX = 6;
/** Beyond this many countries the tooltip lists balances only, so it fits the window. */
const CEILING_ROWS_MAX = 4;

/** Which balance a widget charts, and how it names it. */
export interface MeasureConfig {
  measure: Measure;
  /** The chart's name, such as "Political power". */
  title: string;
  /** The measure in running text, such as "political power". */
  noun: string;
  /** The unit a single country's end label names, such as "PP" (mock-up politics-1). */
  unit: string;
  /** The widget's settings, when it adds to the shared ones. */
  options?: readonly OptionField[];
  /** Draws the recorded maximum dashed (command power), named in running text. */
  ceiling?: string;
}

/** A recorded balance over time (political power, command power), one line per
 * country, with net-spend ticks. Daily income is not recorded, so spends are the
 * falls in the balance. */
export default function MeasureWidget({ widget, runtime, active, headerSlot, onDiagnostics, config }: WidgetProps & { config: MeasureConfig }) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(config.options ?? MEASURE_OPTIONS, widget.options);
  const showCeiling = config.ceiling != null && options.ceiling !== false;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER), [widget.countries, fallback, groups.data, lastHour]);
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-measure',
    params: { branchId: track.branch.id, measure: config.measure, tags: resolved.tags } } : null;
  const { data, error, retry } = useProduct<MeasureReply>(runtime.client, request, active, validateMeasureReply);
  // The maximum comes from the recorded inputs; it never holds up the balance lines.
  const ceilingRequest = showCeiling && track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-stability',
    params: { branchId: track.branch.id, tags: resolved.tags, hour: null } } : null;
  const ceiling = useProduct<CountryStabilityReply>(runtime.client, ceilingRequest, active, validateStabilityReply);

  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  // Names and colours as they were at the moment shown: the playhead, the end of a
  // date range, or the branch end.
  const styleAt = playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour);
  const styles = useCountryLineStyles(runtime.client, track, resolved.tags, styleAt, active);
  const known = styles.known;

  useEffect(() => {
    const uncoloured = uncolouredWarning((data?.series ?? []).map(item => item.tag), known, !!styles.data);
    const warnings = [...(data?.warnings ?? []), ...(styles.data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(ceilingRequest && ceiling.error ? [`The ${config.ceiling} could not be loaded: ${ceiling.error}`] : []),
      ...(ceilingRequest ? ceiling.data?.warnings ?? [] : []),
      ...(uncoloured ? [uncoloured] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, styles.data, known, groups.data, onDiagnostics, ceilingRequest != null, ceiling.error, ceiling.data, config.ceiling]);

  const series = useMemo(() => {
    if (!data) return [];
    const colours = countryLineColours(data.series.map(item => item.tag), known, tokens.surface, tokens.countryColours === 'fixed');
    return data.series.map(item => {
      const style = known.get(item.tag);
      const membership = resolved.membership?.get(item.tag);
      const runs = toRuns(item, data.decimals);
      // A country counted at each hour shows nothing (line, value, spend) outside its membership.
      const drops = item.drops.filter(drop => isMember(membership, drop.hour));
      const valueAt = (hour: number) => isMember(membership, hour) ? rawAt(item, hour) : null;
      // The recorded maximum, clipped to the same membership as the balance.
      const maximum = showCeiling ? ceiling.data?.series.find(entry => entry.tag === item.tag) : undefined;
      const ceilingRuns = maximum && ceiling.data ? toRuns({ runs: maximum.maxCommandPower }, ceiling.data.decimals) : [];
      const ceilingAt = (hour: number) => maximum && isMember(membership, hour) ? rawAt({ runs: maximum.maxCommandPower }, hour) : null;
      // The read a shown maximum comes from: an older one is carried, so its time is shown.
      const ceilingReadAt = (hour: number) => maximum ? readHourAt(maximum.carried, hour) : null;
      return { tag: item.tag, name: style?.name ?? track?.source.metadata.countries.find(country => country.tag === item.tag)?.name ?? item.tag,
        hex: colours.get(item.tag)!, drops, valueAt, runs: membership ? clipRuns(runs, membership) : runs, membership,
        ceilingRuns: membership ? clipRuns(ceilingRuns, membership) : ceilingRuns, ceilingAt, ceilingReadAt };
    });
  }, [data, known, resolved.membership, tokens, track, showCeiling, ceiling.data]);

  // Joins and leaves of a group counted at each hour, as badges in the country's colour.
  const badges = useMemo<EventBadge[]>(() => series.flatMap(item => item.membership
    ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({
      hour: change.hour, glyph: change.change === 'joined' ? '+' : '−', label: `${item.name} ${change.change}`, color: item.hex })) : []),
  [series, extent[0], extent[1], resolved.unobserved]);
  const menu = useContextMenu('Chart actions');
  const head = playhead ?? extent[1];
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  // The last hour drawn: the playhead, unless whole-campaign playback shows past it.
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);

  const layers = useMemo<ChartLayer[]>(() => {
    const single = series.length === 1;
    const list: ChartLayer[] = series.map(item => ({
      draw: (ctx, frame, chartTokens, state) => { drawLine(ctx, frame, chartTokens, item.runs, item.hex, { area: single && options.areaFill === true, until: state.playhead ?? undefined }); },
      extent: (x: readonly [number, number]) => extentOf(item.runs as Run[], x[0], Math.min(x[1], until ?? x[1])),
    }));
    // Each country's maximum, dashed in its colour.
    for (const item of series) if (item.ceilingRuns.length) list.push({
      draw: (ctx, frame, chartTokens, state) => { drawLine(ctx, frame, chartTokens, item.ceilingRuns, item.hex, { dash: SECONDARY_DASH, until: state.playhead ?? undefined }); },
      extent: (x: readonly [number, number]) => extentOf(item.ceilingRuns as Run[], x[0], Math.min(x[1], until ?? x[1])),
    });
    if (options.spendTicks) list.push({ draw: (ctx, frame, chartTokens, state) => {
      for (const item of series) drawEventTicks(ctx, frame, chartTokens, item.drops.map(drop => drop.hour).filter(hour => state.playhead == null || hour <= state.playhead), single ? chartTokens.muted : item.hex);
    } });
    return list;
  }, [series, options.areaFill, options.spendTicks, until]);

  if (!track) return <WidgetState kind="empty" icon="sparkle" message={`Load a campaign to see ${config.noun}.`} />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (widget.countries.kind === 'group' && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label={`Loading ${config.noun}…`} />;
  if (series.every(item => !item.runs.length)) return <WidgetState kind="unavailable" message={`No ${config.noun} was recorded for these countries in this branch.`} />;

  const decimals = data.decimals;
  const currentRaw = series.length === 1 ? series[0].valueAt(Math.floor(head)) : null;
  const hoursPerPixel = (frameWidth: number, span: number) => span / Math.max(1, frameWidth);
  return <div className="country-measure">
    {headerSlot && currentRaw != null && createPortal(<span className="pill">{formatBalance(currentRaw, decimals)}</span>, headerSlot)}
    <Legend label="Countries" items={series.map(item => {
      const raw = item.valueAt(Math.floor(head));
      return { id: item.tag, label: item.name, color: item.hex, value: raw == null ? 'Not recorded' : formatBalance(raw, decimals),
        flag: <SourceFlag client={runtime.client} source={track.source} tag={item.tag} size="shield" /> };
    })} />
    {/* The dashed line is named, so it never rests on its dash pattern alone. */}
    {series.some(item => item.ceilingRuns.length) && <p className="chart-line-key"><i className="is-dashed" aria-hidden="true" />Dashed line: {config.ceiling}</p>}
    <div className="country-measure-chart">
      <TimeChart label={`${config.title} of ${series.map(item => item.name).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        padding={{ right: options.endLabels ? CHART_LAYOUT.measureEndLabels : CHART_LAYOUT.right }} layers={layers} active={active} revision={`${head}:${data.branchId}:${series.length}:${viewport}`}
        formatValue={value => Math.abs(value) >= 1000 ? `${Math.round(value / 100) / 10}k` : String(Math.round(value))}
        snap={(hour, frame) => {
          if (!options.spendTicks) return hour;
          const tolerance = SNAP_PX * hoursPerPixel(frame.plotWidth, frame.x[1] - frame.x[0]);
          // Only ticks that are drawn: inside the view and not after the last hour drawn.
          const last = Math.min(frame.x[1], until ?? frame.x[1]);
          const visible = (drop: { hour: number }) => drop.hour >= frame.x[0] && drop.hour <= last;
          const nearest = series.map(item => nearestDrop(item.drops.filter(visible), hour, tolerance)).filter(drop => drop != null)
            .sort((a, b) => Math.abs(a!.hour - hour) - Math.abs(b!.hour - hour))[0];
          return nearest ? nearest.hour : hour;
        }}
        badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hovered) => {
          // Without maximum rows, maximums carried from an earlier read are still named.
          const carriedNote = (at: number) => {
            const carried = series.flatMap(item => {
              const read = item.ceilingAt(at) != null ? item.ceilingReadAt(at) : null;
              return read != null && read < at ? [`${item.name} (${formatDateTime(read)})`] : [];
            });
            if (!carried.length) return [];
            const listed = carried.slice(0, CEILING_ROWS_MAX).join(', ');
            return [`Maximums read earlier: ${listed}${carried.length > CEILING_ROWS_MAX ? ` and ${carried.length - CEILING_ROWS_MAX} more` : ''}.`];
          };
          const rows: TooltipRow[] = series.flatMap(item => {
            const raw = item.valueAt(hour);
            const balance: TooltipRow = { label: item.name, swatch: item.hex, value: raw == null ? 'Not recorded' : formatBalance(raw, decimals), tone: raw == null ? 'muted' : undefined };
            if (!showCeiling || !ceiling.data || series.length > CEILING_ROWS_MAX) return [balance];
            const maximum = item.ceilingAt(hour);
            const read = item.ceilingReadAt(hour);
            // A carried maximum always names its read, even an hour with no calendar date.
            const carried = maximum != null && read != null && read < hour ? ` (read ${formatDateTime(read)})` : '';
            return [balance, { label: series.length > 1 ? `${item.name}: maximum` : 'Maximum', value: maximum == null ? 'Not recorded' : `${formatBalance(maximum, ceiling.data.decimals)}${carried}`, tone: 'muted' }];
          });
          for (const item of series) {
            const drop = item.drops.find(entry => entry.hour === hour);
            if (drop) rows.push({ label: `${series.length > 1 ? `${item.name}: ` : ''}Spent (net)`, value: netSpend(drop, decimals), tone: 'negative', divider: true });
          }
          rows.push(...badgeRows(hovered));
          const notes = [
            ...(rows.some(row => row.tone === 'negative') ? ['Daily income is not recorded, so a spend is the fall in the balance.'] : []),
            ...(showCeiling && ceiling.data && series.length > CEILING_ROWS_MAX ? [`Maximums are drawn dashed; they are listed here for up to ${CEILING_ROWS_MAX} countries.`, ...carriedNote(hour)] : []),
          ];
          const content = { title: dayLabel(hour), rows, note: notes.length ? notes.join(' ') : undefined };
          return series.length > 1 ? fitTooltip([content], viewport) : content;
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          // At the end of the lines drawn, never past the recorded hours.
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          const ends = series.flatMap(item => {
            // Where the line ends, even when the playhead or view end lies in a gap after it.
            const last = lineEndHour(item.runs, frame.x[0], at);
            const raw = last == null ? null : item.valueAt(last);
            if (raw == null) return [];
            // One country: its value and unit, as signed off; several: each named.
            const text = series.length === 1 ? `${formatFixed(raw, decimals, 1)} ${config.unit}` : `${item.name} ${formatBalance(raw, decimals)}`;
            return [{ id: item.tag, y: frame.toY(Number(raw) / 10 ** decimals), text, color: item.hex }];
          });
          placeEndLabels(ctx, frame, chartTokens, ends);
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {menu.menu}
  </div>;
}
