import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import RegionProvinces from './RegionProvinces';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { useContextMenu } from '../../components/ContextMenu';
import HeatChart from '../../charts/heat/HeatChart';
import type { HeatPeriod } from '../../charts/heat/layout';
import { signedColor } from '../../charts/heat/matrix';
import { hatchStyle } from '../../charts/hatch';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet } from '../../dashboard/options';
import { theatreRegions, useAirTheatres } from '../../dashboard/airTheatres.ts';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { windowExtent } from '../../dashboard/widgetRequests';
import { mix } from '../../shell/appearance/color';
import { useHeadHour } from '../army-size/headHour.ts';
import { SHOWS } from './definition';
import { columnTooltip, pickedDay, shownRegions, uncoveredSpans, weatherPeriods, weatherRows } from './weatherData';
import type { Choice, WeatherPeriod, WeatherShow } from './weatherData';
import { useWeatherInView } from './useWeather';
import './weather.css';

/** Recorded weather by strategic region and day: conditions as stripes, the regions' mean temperature, or one recorded
 * weather modifier on a ramp. Long views fold their days into the plot's pixel columns. Following the playhead it
 * renders only when the window shown changes; the playhead is drawn at the clock each frame. */
export default function WeatherWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = active && widget.time.mode === 'follow';
  const extentAt = (hour: number) => { const window = track && resolveWindow(widget, track, runtime.tracks, hour); return track && window ? windowExtent(track, window) : null; };
  const playback = useHeadHour(runtime.clock, following, hour => JSON.stringify(extentAt(hour)));
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Weather by region');
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const extent: [number, number] | null = track && window ? windowExtent(track, window) : null;
  const axisOffset = track?.axisOffset ?? 0;
  const clockHour = () => track ? Math.max(track.branch.firstHour, Math.min(Math.floor(runtime.clock.getHour()) + axisOffset, track.branch.lastHour)) : null;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const { options, show, modifier, data, error, retry, onSummaryView } = useWeatherInView(widget, runtime, active, extent);
  useShownEachFrame(runtime.clock, () => data ? following ? Math.min(clockHour() ?? data.lastHour, data.lastHour) : data.lastHour : null);
  const dayHours = data?.dayHours ?? 1;
  const chosenTheatres = useMemo(() => contentSet(options.theatres).map(contentKey), [options.theatres]);
  // The theatres' regions at the hour shown; a theatre filter whose membership is not known shows why instead.
  const air = useAirTheatres(track, extent?.[1] ?? null, active && chosenTheatres.length > 0);
  const inTheatres = useMemo(() => theatreRegions(air.theatres, chosenTheatres), [air.theatres, chosenTheatres]);
  const regions = useMemo(() => data ? shownRegions(data, contentSet(options.regions).map(contentKey), inTheatres) : [], [data, options.regions, inTheatres]);
  const periods = useMemo(() => data && extent ? weatherPeriods(data.columns, extent, data.dayHours) : [], [data, extent?.[0], extent?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  // Hours the shown reply does not reach while the next loads: never clear days.
  const unreached = useMemo(() => data && extent ? uncoveredSpans(extent, data) : [], [data, extent?.[0], extent?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const grey = mix(tokens.surface, tokens.text, 0.1);
  const gap = `color-mix(in srgb, ${tokens.error} ${tokens.gapOpacity * 100}%, ${tokens.surface})`;
  const flagColour = useCallback((index: number) => tokens.categories[index % tokens.categories.length], [tokens.categories]);
  const ramp = useCallback((value: number, highest: number) => signedColor(value, 0, highest, grey, grey, tokens.accent), [grey, tokens.accent]);
  const temperature = useCallback((value: number, low: number, high: number) => signedColor(value, low, high, grey, tokens.accent, tokens.error), [grey, tokens.accent, tokens.error]);
  const heat = useMemo(() => data ? weatherRows(data, regions, periods, show, modifier, { flag: flagColour, ramp, temperature, clear: tokens.surface, gap }) : null,
    [data, regions, periods, show, modifier, flagColour, ramp, temperature, tokens.surface, gap]);
  const legend = useMemo(() => show === 'conditions' ? (data?.flags ?? []).map((flag, index) => ({ id: flag.id, label: flag.name, color: flagColour(index) }))
    : show === 'temperature' ? [{ id: 'cold', label: 'Colder', color: tokens.accent }, { id: 'warm', label: 'Warmer', color: tokens.error }]
    : show === 'modifiers' && data ? [{ id: 'modifier', label: data.modifiers.find(item => item.id === modifier)?.name ?? 'Modifier', color: tokens.accent }] : [],
    [show, data, modifier, flagColour, tokens.accent, tokens.error]);
  const tooltip = useCallback((row: { id: string }, period: HeatPeriod | null, hour: number): TooltipContent | null => {
    const region: Choice | undefined = regions.find(item => item.id === row.id);
    if (!data || !region) return null;
    const date = dayLabel(Math.floor(hour));
    if (unreached.some(span => span.firstHour <= hour && hour <= span.lastHour)) return { title: region.name, rows: [{ label: date }, { label: 'Loading…' }] };
    const shown = period as WeatherPeriod | null;
    if (!shown) return { title: region.name, rows: [{ label: date }, { label: 'Not recorded' }] };
    return columnTooltip(data, region, shown.column, show, modifier, date, dayLabel);
  }, [data, regions, show, modifier, unreached]);
  const onMenu = useCallback((hour: number, x: number, y: number) => {
    const period = periods.find(item => item.firstHour <= hour && hour <= item.lastHour);
    if (track) menu.open(x, y, [jumpItem(runtime, track.id, period?.exact === false ? hour : period?.readHour ?? hour)]);
  }, [track, periods, menu.open, runtime]); // eslint-disable-line react-hooks/exhaustive-deps
  const anchor = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState<{ id: string; name: string } | null>(null);
  const [drillWarnings, setDrillWarnings] = useState<readonly string[]>([]);
  // The read the drill's provinces answer.
  const [drillShown, setDrillShown] = useState<number | null>(null);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...data.temperature === 'available' ? [] : [data.temperature], ...drillWarnings] : null);
  if (!track) return <WidgetState kind="empty" icon="sun" message="Load a campaign to see weather." />;
  if (!window || !extent || extent[1] < extent[0]) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading weather…" />;
  const caption = <div className="weather-caption">
    <div className="weather-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: tokens.surface }} />{show === 'conditions' ? 'No condition' : show === 'temperature' ? 'No average' : 'No modifier'}</span>
      <span><i aria-hidden="true" style={{ background: gap }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />Unknown</span>
    </div>
    {setOption && <Segmented value={show} options={SHOWS} label="Show" onChange={value => setOption('show', value)} />}
  </div>;
  if (chosenTheatres.length && !air.theatres) return <div className="weather">{caption}<WidgetState kind="unavailable" message={air.unavailable ?? 'Air theatres are not known at this time.'} /></div>;
  if (!heat || !heat.rows.length) return <div className="weather">{caption}<WidgetState kind="empty" message="No regions match these filters." /></div>;
  const chart = <div className="weather-chart"><HeatChart label="Weather by region" rows={heat.rows} periods={periods} labelCatalogue={regions.map(region => region.name)}
      extent={extent} playhead={widget.time.mode === 'follow' ? Math.min(clockHour() ?? extent[1], extent[1]) : null} liveHead={liveHead} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={widget.time.mode !== 'follow'} look="split" active={active} revision={`${show}:${modifier}:${data.lastHour}`}
      hoursPerSecond={hoursPerSecond} loading={unreached} tooltip={tooltip} onContextMenu={onMenu} onSummaryView={onSummaryView}
      pickOwner={track.source.id} onPick={(row, _period, hour) => {
        const region = regions.find(item => item.id === row.id);
        if (!region) return;
        setPicked(region);
        const playing = widget.time.mode === 'follow';
        // The provinces of the day under the pointer, whatever its column holds.
        dispatch({ type: 'open', breakdown: 'provinces', hour: pickedDay(hour, dayHours, extent), playing, playhead: playing ? clockHour() : null });
      }} /></div>;
  const pinned = drill.breakdown !== null && !drill.follow ? drill.hour : null;
  return <div ref={anchor} className="weather">
    {caption}
    <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null}
      title={picked?.name ?? 'Region'} day={drillShown === null ? '' : dayLabel(drillShown)} following={drill.follow} canFollow
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={chart}>
      {picked && drill.breakdown !== null && <RegionProvinces runtime={runtime} widget={widget} region={picked.id} hour={pinned} active={active} onWarnings={setDrillWarnings} onHour={setDrillShown} />}
    </DrillHost>
    {menu.menu}
  </div>;
}

export type { WeatherShow };
