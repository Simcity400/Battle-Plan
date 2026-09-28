import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel, drawLine, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { SECONDARY_DASH } from '../../charts/palette';
import { extentOfAll, lineEndHour, runAt } from '../../charts/series';
import { barStrip } from '../../charts/strip';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, ChartStrip } from '../../charts/TimeChart';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { FuelReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { formatFuel, fuelRuns, fuelTooltip, fuelView, validateFuelReply } from './fuelData';
import './fuel.css';

const NO_WARNINGS: string[] = [];
/** The gain and burn strip's share of the chart's height, and its least height (mock-up economy-1). */
const STRIP_SHARE = 0.28;
const STRIP_MIN = 40;
/** Room for the end labels at the right of the plot. */
const END_LABEL_ROOM = 128;

/** One country's fuel over time (mock-up economy-1): the level as a filled line under
 * its capacity, dashed, with each day's gain (up) and burn (down) in a strip under the
 * chart on the same time axis. */
export default function FuelWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-fuel',
    params: { branchId: track.branch.id, tag } } : null;
  const { data, error, retry } = useProduct<FuelReply>(runtime.client, request, active, validateFuelReply);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  // The last hour drawn: the playhead, unless whole-campaign playback shows past it.
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const menu = useContextMenu('Fuel actions');
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  const colors = { fuel: tokens.accent, capacity: tokens.muted, gained: tokens.categories[0], burned: tokens.categories[1] };
  const showCapacity = options.capacity !== false;
  // Only the hours shown (a date range, or the branch): an axis dragged past a range
  // draws, fits and hovers nothing beyond it.
  const view = useMemo(() => data ? fuelView(data, extent[0], extent[1]) : null, [data, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => data && view ? { fuel: fuelRuns(view.reads, data.quantityScale, 'fuel'), capacity: fuelRuns(view.reads, data.quantityScale, 'capacity') } : null, [data, view]);
  const layers = useMemo<ChartLayer[]>(() => {
    if (!lines) return [];
    const fitted = showCapacity ? [lines.fuel, lines.capacity] : [lines.fuel];
    return [{
      draw: (ctx, frame, chartTokens, state) => {
        if (showCapacity) drawLine(ctx, frame, chartTokens, lines.capacity, chartTokens.muted, { dash: SECONDARY_DASH, endDot: false, until: state.playhead ?? undefined });
        drawLine(ctx, frame, chartTokens, lines.fuel, chartTokens.accent, { area: options.areaFill !== false, until: state.playhead ?? undefined });
      },
      extent: x => extentOfAll(fitted, x[0], Math.min(x[1], until ?? x[1])),
    }];
  }, [lines, showCapacity, options.areaFill, until]);
  const bars = view?.bars;
  // Hovering a bar shows the read of the bar drawn there; the fuel line keeps its own hover.
  const strip = useMemo<ChartStrip | undefined>(() => options.strip === false || !bars ? undefined
    : barStrip(bars, { height: chartHeight => Math.max(STRIP_MIN, (chartHeight - 50) * STRIP_SHARE), upLabel: 'gain', downLabel: 'burn' }), [bars, options.strip]);

  if (!track || !tag) return <WidgetState kind="empty" icon="droplet" message="Load a campaign and choose a country to see its fuel." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || !view) return <WidgetState kind="loading" label="Loading fuel…" />;
  if (!data.reads.length) return <WidgetState kind="unavailable" message="No fuel was recorded for this country in this branch." />;

  const name = track.source.metadata.countries.find(country => country.tag === tag)?.name ?? tag;
  const head = Math.floor(playhead ?? extent[1]);
  return <div className="fuel">
    {/* Without end labels the dashed line is named, so it never rests on its dash pattern alone. */}
    {!options.endLabels && showCapacity && <p className="chart-line-key"><i aria-hidden="true" />Solid: fuel · <i className="is-dashed" aria-hidden="true" />dashed: capacity</p>}
    <div className="fuel-chart">
      <TimeChart label={`Fuel of ${name}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        padding={{ right: options.endLabels ? END_LABEL_ROOM : 14 }} layers={layers} strip={strip} keyHours={view.readHours}
        active={active} revision={`${head}:${data.branchId}:${view.reads.length}:${viewport}`}
        tooltip={hour => hour < extent[0] || hour > extent[1] ? null
          : fuelTooltip(data, view.reads, view.readHours, hour, dayLabel(hour), colors, viewport)}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          // At the end of the lines drawn, never past the recorded hours, even when the
          // playhead or the view ends in a gap after the last read.
          const last = lineEndHour(view.reads, frame.x[0], Math.min(until ?? extent[1], frame.x[1], extent[1]));
          const read = last == null ? null : runAt(view.reads, last);
          if (!read) return;
          const ends = [
            ...(showCapacity ? [{ id: 'capacity', raw: read.capacityRaw, label: 'Capacity', color: chartTokens.muted, muted: true }] : []),
            { id: 'fuel', raw: read.fuelRaw, label: 'Fuel', color: chartTokens.accent, muted: false },
          ];
          placeEndLabels(ctx, frame, chartTokens, ends.map(end => ({ id: end.id, color: end.color, muted: end.muted,
            y: frame.toY(Number(end.raw) / data.quantityScale), text: `${end.label} ${formatFuel(end.raw, data.quantityScale, 0)}` })));
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {menu.menu}
  </div>;
}
