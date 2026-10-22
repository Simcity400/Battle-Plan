import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import RegionProvinces from './RegionProvinces';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { useContextMenu } from '../../components/ContextMenu';
import HeatChart from '../../charts/heat/HeatChart';
import { signedColor } from '../../charts/heat/matrix';
import { hatchStyle } from '../../charts/hatch';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import { keyedDays } from '../../dashboard/keyedDays';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { mix } from '../../shell/appearance/color';
import { SHOWS } from './definition';
import { shownRegions, weatherRows, weatherTooltip } from './weatherData';
import type { WeatherShow } from './weatherData';
import { useWeather } from './useWeather';
import { windowExtent } from '../../dashboard/widgetRequests';
import '../air-superiority-timeline/airSuperiorityTimeline.css';
import './weather.css';

/** Recorded weather by strategic region and day, in the air superiority timeline's form:
 * conditions as stripes, or one recorded weather modifier on a ramp. */
export default function WeatherWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const playback = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Weather by region');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const extent: [number, number] | null = track && window ? windowExtent(track, window) : null;
  const { options, show, modifier, data, error, retry } = useWeather(widget, runtime, active, extent);
  const regions = useMemo(() => data ? shownRegions(data, contentSet(options.regions).map(contentKey), contentSet(options.theatres).map(contentKey)) : [],
    [data, options.regions, options.theatres]);
  const days = useMemo(() => data && extent ? keyedDays(data.window, extent, data.dayHours) : [], [data, extent?.[0], extent?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const grey = mix(tokens.surface, tokens.text, 0.1);
  const flagColour = useCallback((index: number) => tokens.categories[index % tokens.categories.length], [tokens.categories]);
  const ramp = useCallback((value: number, highest: number) => signedColor(value, 0, highest, grey, grey, tokens.accent), [grey, tokens.accent]);
  const temperature = useCallback((value: number, low: number, high: number) => signedColor(value, low, high, grey, tokens.accent, tokens.error), [grey, tokens.accent, tokens.error]);
  const heat = useMemo(() => data ? weatherRows(data, regions, days, show, modifier, { flag: flagColour, ramp, temperature }) : null,
    [data, regions, days, show, modifier, flagColour, ramp, temperature]);
  const legend = useMemo(() => show === 'conditions' ? (data?.flags ?? []).map((flag, index) => ({ id: flag.id, label: flag.name, color: flagColour(index) }))
    : show === 'temperature' ? [{ id: 'cold', label: 'Colder', color: tokens.accent }, { id: 'warm', label: 'Warmer', color: tokens.error }]
    : show === 'modifiers' && data ? [{ id: 'modifier', label: data.modifiers.find(item => item.id === modifier)?.name ?? 'Modifier', color: tokens.accent }] : [],
    [show, data, modifier, flagColour, tokens.accent, tokens.error]);
  const dayAt = useCallback((hour: number) => days.find(day => day.firstHour <= hour && hour <= day.lastHour) ?? null, [days]);
  const tooltip = useCallback((row: { id: string }, _period: unknown, hour: number) => {
    const region = regions.find(item => item.id === row.id);
    if (!data || !region) return null;
    const day = dayAt(hour);
    return weatherTooltip(data, region, day, dayLabel(Math.floor(hour)), day?.readHour == null ? null : dayLabel(day.readHour));
  }, [data, regions, dayAt]);
  const onMenu = useCallback((hour: number, x: number, y: number) => {
    const read = dayAt(hour)?.readHour;
    if (track) menu.open(x, y, [jumpItem(runtime, track.id, read ?? hour)]);
  }, [track, dayAt, menu.open, runtime]);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [picked, setPicked] = useState<{ id: string; name: string } | null>(null);
  const [drillWarnings, setDrillWarnings] = useState<readonly string[]>([]);
  const drillAt = drillHour(drill, Math.min(playback, extent?.[1] ?? playback));
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...data.temperature === 'available' ? [] : [data.temperature], ...drillWarnings] : null);
  if (!track) return <WidgetState kind="empty" icon="sun" message="Load a campaign to see weather." />;
  if (!window || !extent || extent[1] < extent[0]) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading weather…" />;
  const prefix = preparedPrefix(data);
  const caption = <div className="weather-caption">
    <div className="air-pattern-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: tokens.surface }} />{show === 'conditions' ? 'No condition' : show === 'temperature' ? 'No average' : 'No modifier'}</span>
      <span><i aria-hidden="true" style={{ background: `color-mix(in srgb, ${tokens.error} ${tokens.gapOpacity * 100}%, ${tokens.surface})` }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />Unknown</span>
    </div>
    {setOption && <Segmented value={show} options={SHOWS} label="Show" onChange={value => setOption('show', value)} />}
  </div>;
  if (!heat || !heat.rows.length) return <div className="air-timeline weather">{caption}<WidgetState kind="empty" message="No regions match these filters." /></div>;
  const chart = <div className="air-timeline-chart"><HeatChart label="Weather by region" rows={heat.rows} periods={days} labelCatalogue={regions.map(region => region.name)}
      extent={extent} playhead={widget.time.mode === 'follow' ? Math.min(playback, extent[1]) : null} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={widget.time.mode !== 'follow'} look="split" active={active} revision={`${show}:${modifier}:${data.window.lastHour}`}
      hoursPerSecond={hoursPerSecond} pending={prefix?.pending} tooltip={tooltip} onContextMenu={onMenu}
      pickOwner={track.source.id} onPick={(row, _period, hour) => {
        const region = regions.find(item => item.id === row.id);
        if (!region) return;
        setPicked(region);
        dispatch({ type: 'open', breakdown: 'provinces', hour: dayAt(hour)?.at ?? hour, playing: widget.time.mode === 'follow', playhead: Math.min(playback, extent[1]) });
      }} /></div>;
  return <div ref={anchor} className="air-timeline weather">
    {caption}
    <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null}
      title={picked?.name ?? 'Region'} day={drillAt === null ? '' : dayLabel(drillAt)} following={drill.follow} canFollow
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={chart}>
      {picked && drillAt !== null && <RegionProvinces runtime={runtime} track={track} region={picked.id} hour={drillAt} active={active} onWarnings={setDrillWarnings} />}
    </DrillHost>
    {menu.menu}
  </div>;
}

export type { WeatherShow };
