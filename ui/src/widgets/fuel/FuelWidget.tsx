import { useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { SECONDARY_DASH } from '../../charts/palette';
import CountryRaceView from '../../charts/race/CountryRaceView';
import type { CountryRaceLine } from '../../charts/race/countryRace';
import { runAt } from '../../charts/series';
import { barStrip } from '../../charts/strip';
import type { ChartStrip } from '../../charts/TimeChart';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens } from '../../charts/useChartTokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { FuelReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { formatFuel, fuelRuns, fuelTooltips, fuelView, validateFuelReply } from './fuelData';

const NO_WARNINGS: string[] = [];
/** The gain and burn strip's share of the chart's height (mock-up economy-1). */
const STRIP_SHARE = 0.28;

/** One country's fuel level and capacity, with its daily gain and burn strip. */
export default function FuelWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
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
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  const colors = { fuel: tokens.accent, capacity: tokens.muted, gained: tokens.categories[0], burned: tokens.categories[1] };
  const showCapacity = options.capacity !== false;
  const view = useMemo(() => data ? fuelView(data, extent[0], extent[1]) : null, [data, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo<CountryRaceLine[]>(() => {
    if (!data || !view) return [];
    const line = (of: 'fuel' | 'capacity'): CountryRaceLine => ({
      tag: of, runs: fuelRuns(view.reads, data.quantityScale, of),
      color: of === 'fuel' ? tokens.accent : tokens.muted,
      ...(of === 'capacity' ? { dash: SECONDARY_DASH, endDot: false, muted: true } : { area: options.areaFill !== false, muted: false }),
      endLabel: hour => {
        const read = runAt(view.reads, hour)!;
        return `${of === 'fuel' ? 'Fuel' : 'Capacity'} ${formatFuel(of === 'fuel' ? read.fuelRaw : read.capacityRaw, data.quantityScale, 0)}`;
      },
    });
    return [...(showCapacity ? [line('capacity')] : []), line('fuel')];
  }, [data, view, showCapacity, options.areaFill, tokens]);
  const bars = view?.bars;
  // The strip's hover stays about its read while the level keeps its own hover.
  const strip = useMemo<ChartStrip | undefined>(() => options.strip === false || !bars ? undefined
    : barStrip(bars, { height: chartHeight => Math.max(CHART_LAYOUT.stripMin, (chartHeight - CHART_LAYOUT.stripInset) * STRIP_SHARE),
      upLabel: 'gain', downLabel: 'burn' }), [bars, options.strip]);

  const state = (() => {
    if (!track || !tag) return <WidgetState kind="empty" icon="droplet" message="Load a campaign and choose a country to see its fuel." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
    if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
    if (!data || !view) return <WidgetState kind="loading" label="Loading fuel…" />;
    if (!data.reads.length) return <WidgetState kind="unavailable" message="No fuel was recorded for this country in this branch." />;
  })();

  const name = track?.source.metadata.countries.find(country => country.tag === tag)?.name ?? tag;
  return <CountryRaceView runtime={runtime} track={track} active={active} options={options} view="lines" extent={extent} playhead={playhead}
    state={state} moment={Math.floor(playhead ?? extent[1])} slots={1} ranked={[]} lines={lines} strip={strip} keyHours={view?.readHours}
    label={`Fuel of ${name}`} labels={{ lines: `Fuel of ${name}`, actions: 'Fuel actions' }} empty="" formatValue={String}
    presentation={{ countryLooks: false, held: false, linePadding: CHART_LAYOUT.measureEndLabels }}
    beforeChart={!options.endLabels && showCapacity
      ? <p className="chart-line-key"><i aria-hidden="true" />Solid: fuel · <i className="is-dashed" aria-hidden="true" />dashed: capacity</p> : null}
    tooltip={() => null}
    timeTooltip={hour => data && view ? fuelTooltips(data, view.reads, view.readHours, hour, dayLabel(hour), colors) : null} />;
}
