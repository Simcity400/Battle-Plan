import { useCallback, useMemo,  useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import RegionProvinces from './RegionProvinces';
import WidgetState from '../../components/WidgetState';
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
import { columnTooltip, pickedDay, shownRegions, uncoveredSpans, weatherPeriods, weatherRows } from './weatherData';
import type { Choice, WeatherPeriod, WeatherShow } from './weatherData';
import { useWeatherInView } from './useWeather';
import './weather.css';
import {  recordedHour } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';

export default function WeatherWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const drillTime = drillControl.region.action;
  const following = active && widget.time.mode === 'follow';
  const extentAt = (hour: number) => { const window = track && resolveWindow(widget, track, runtime.tracks, hour); return track && window ? windowExtent(track, window) : null; };
  const playback = useHeadHour(runtime.clock, following, hour => JSON.stringify(extentAt(hour)));
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Weather by region');
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const extent: [number, number] | null = track && window ? windowExtent(track, window) : null;
  const axisOffset = track?.axisOffset ?? 0;
  const clockHour = () => recordedHour(track, Math.floor(runtime.clock.getHour()) + axisOffset);
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const { options, show, modifier, data, error, retry, onSummaryView } = useWeatherInView(widget, runtime, active, extent);
  useShownEachFrame(runtime.clock, () => data ? following ? clockHour() : data.lastHour : null);
  const dayHours = data?.dayHours ?? 1;
  const chosenTheatres = useMemo(() => contentSet(options.theatres).map(contentKey), [options.theatres]);
  const air = useAirTheatres(track, extent?.[1] ?? null, active && chosenTheatres.length > 0);
  const inTheatres = useMemo(() => theatreRegions(air.theatres, chosenTheatres), [air.theatres, chosenTheatres]);
  const regions = useMemo(() => data ? shownRegions(data, contentSet(options.regions).map(contentKey), inTheatres) : [], [data, options.regions, inTheatres]);
  const periods = useMemo(() => data && extent ? weatherPeriods(data.columns, extent, data.dayHours) : [], [data, extent?.[0], extent?.[1]]);
  const unreached = useMemo(() => data && extent ? uncoveredSpans(extent, data) : [], [data, extent?.[0], extent?.[1]]);
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
  }, [track, periods, menu.open, runtime]);
  const anchor = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState<{ id: string; name: string } | null>(null);
  const [drillWarnings, setDrillWarnings] = useState<readonly string[]>([]);
  const [drillShown, setDrillShown] = useState<number | null>(null);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...data.temperature === 'available' ? [] : [data.temperature], ...drillWarnings] : null);
  if (!track) return <WidgetState kind="empty" icon="sun" message="Load a campaign to see weather." />;
  const caption = data && <div className="weather-caption">
    <div className="weather-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: tokens.surface }} />{show === 'conditions' ? 'No condition' : show === 'temperature' ? 'No average' : 'No modifier'}</span>
      <span><i aria-hidden="true" style={{ background: gap }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />Unknown</span>
    </div>
  </div>;
  const chart = !window || !extent || extent[1] < extent[0] ? <WidgetState kind="outside" />
    : error ? <WidgetState kind="error" failure={error} onRetry={retry} />
    : !data ? <WidgetState kind="loading" label="Loading weather…" />
    : chosenTheatres.length && !air.theatres ? <WidgetState kind="unavailable" message={air.unavailable ?? 'Air theatres are not known at this time.'} />
    : !heat || !heat.rows.length ? <WidgetState kind="empty" message="No regions match these filters." />
    : <div className="weather-chart"><HeatChart label="Weather by region" rows={heat.rows} periods={periods} labelCatalogue={regions.map(region => region.name)}
      extent={extent} playhead={widget.time.mode === 'follow' ? clockHour() : null} liveHead={liveHead} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={widget.time.mode !== 'follow'} look="split" active={active} revision={heat}
      hoursPerSecond={hoursPerSecond} loading={unreached} tooltip={tooltip} onContextMenu={onMenu} onSummaryView={onSummaryView}
      pickOwner={track.source.id} onPick={(row, _period, hour) => {
        const region = regions.find(item => item.id === row.id);
        if (!region) return;
        setPicked(region);
        drillControl.show('provinces', pickedDay(hour, dayHours, extent));
      }} /></div>;
  if (drill.breakdown === null && (!window || !extent || extent[1] < extent[0] || error || !data)) return chart;
  if (drill.breakdown === null && (chosenTheatres.length && !air.theatres || !heat || !heat.rows.length)) return <div className="weather">{caption}{chart}</div>;
  return <div ref={anchor} className="weather">
    {caption}
    <DrillHost drill={drillControl} runtime={runtime} anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title={picked?.name ?? 'Region'} day={drillShown === null ? '' : dayLabel(drillShown)} canFollow
      onClose={() => dispatch({ type: 'close' })} main={chart}>
      {picked && drill.breakdown !== null && <RegionProvinces runtime={runtime} widget={widget} region={picked.id} time={drillTime} active={active} onWarnings={setDrillWarnings} onHour={setDrillShown} />}
    </DrillHost>
    {menu.menu}
  </div>;
}

export type { WeatherShow };
