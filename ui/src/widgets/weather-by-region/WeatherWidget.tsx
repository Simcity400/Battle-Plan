import { failureText } from '../../dashboard/queryError.ts';
import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
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
import { keyedDays } from '../../dashboard/keyedDays';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { finalPreparedReply, pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { mix } from '../../shell/appearance/color';
import { SHOWS } from './definition';
import { columnTooltip, pickedDay, shownRegions, uncoveredSpans, weatherPeriods, weatherRows, weatherTooltip } from './weatherData';
import type { Choice, WeatherPeriod, WeatherShow } from './weatherData';
import { useWeatherDays, useWeatherInView } from './useWeather';
import { windowExtent } from '../../dashboard/widgetRequests';
import '../air-superiority-timeline/airSuperiorityTimeline.css';
import './weather.css';

/** Recorded weather by strategic region and day, in the air superiority timeline's form:
 * conditions as stripes, or one recorded weather modifier on a ramp. Long views fold
 * their days into the plot's pixel columns. */
export default function WeatherWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  // A drill that follows playback follows it whatever the widget's own time.
  const playback = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Weather by region');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const extent: [number, number] | null = track && window ? windowExtent(track, window) : null;
  // Playback in this campaign's hours, not the comparison clock's, within its branch.
  const clockHour = track ? Math.max(track.branch.firstHour, Math.min(Math.floor(playback) + track.axisOffset, track.branch.lastHour)) : null;
  const { options, show, modifier, data, error, retry, onSummaryView } = useWeatherInView(widget, runtime, active, extent);
  const dayHours = data?.dayHours ?? 1;
  const regions = useMemo(() => data ? shownRegions(data, contentSet(options.regions).map(contentKey), contentSet(options.theatres).map(contentKey)) : [],
    [data, options.regions, options.theatres]);
  const periods = useMemo(() => data?.columns && extent ? weatherPeriods(data.columns, extent, data.dayHours) : [], [data, extent?.[0], extent?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  // Hours still being prepared, and those the shown reply does not reach while the next loads: never clear days.
  const unreached = useMemo(() => data && extent ? uncoveredSpans(extent, data.window) : [], [data, extent?.[0], extent?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const pending = useMemo(() => [...data ? preparedPrefix(data)?.pending ?? [] : [], ...unreached].sort((a, b) => a.firstHour - b.firstHour), [data, unreached]);
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
  // A folded column's hovered day, read exactly once the pointer rests on it.
  const days = useWeatherDays(widget, runtime, active, dayHours);
  /** The hovered day of a folded column, as one day of the history reads it. */
  const exactDay = useCallback((hour: number) => {
    const read = days.get(Math.floor(hour / dayHours));
    if (!read?.data || !extent) return read ? { error: read.error } : null;
    const start = Math.floor(hour / dayHours) * dayHours;
    return { reply: read.data, day: keyedDays(read.data.window, [Math.max(start, extent[0]), Math.min(start + dayHours - 1, extent[1])], dayHours)[0] ?? null };
  }, [days, dayHours, extent?.[0], extent?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const tooltip = useCallback((row: { id: string }, period: HeatPeriod | null, hour: number): TooltipContent | null => {
    days.hover(Math.floor(hour / dayHours));
    const region: Choice | undefined = regions.find(item => item.id === row.id);
    if (!data?.columns || !region) return null;
    const date = dayLabel(Math.floor(hour));
    const within = (spans: readonly { firstHour: number; lastHour: number }[]) => spans.some(span => span.firstHour <= hour && hour <= span.lastHour);
    if (within(unreached)) return { title: region.name, rows: [{ label: date }, { label: 'Loading…' }] };
    if (within(pending)) return { title: region.name, rows: [{ label: date }, { label: 'Still being prepared' }] };
    const shown = period as WeatherPeriod | null;
    if (!shown) return weatherTooltip(data, region, null, date, null);
    const summary = columnTooltip(data, region, shown.column, show, modifier, date, dayLabel);
    if (shown.exact !== false) return summary;
    const exact = exactDay(hour);
    // An unread day is read; one read before preparation finished is read again.
    if (!exact || exact.reply && !finalPreparedReply(exact.reply)) days.want(Math.floor(hour / dayHours));
    if (!exact) return { ...summary, rows: [...summary.rows, { label: date, value: 'Loading…', divider: true }] };
    if (!exact.reply) return { ...summary, rows: [...summary.rows, { label: date,
      value: exact.error ? failureText(exact.error, 'Couldn’t load weather.') : 'Loading…', detail: exact.error?.message, divider: true, tone: 'muted' }] };
    if (pendingAt(preparedPrefix(exact.reply), hour)) return { ...summary, rows: [...summary.rows, { label: date, value: 'Still being prepared', divider: true }] };
    // The day reads its values; the shown history names its flags and modifiers.
    const day = weatherTooltip({ ...exact.reply, flags: data.flags, modifiers: data.modifiers }, region, exact.day, date,
      exact.day?.readHour == null ? null : dayLabel(exact.day.readHour), show);
    return { ...summary, rows: [...summary.rows, ...day.rows.map((item, index) => index === 0 ? { ...item, divider: true } : item)] };
  }, [data, regions, show, modifier, exactDay, days, dayHours, pending, unreached]);
  /** The hovered day's own hours: its read where one day is shown, never a folded column's. */
  const readAt = useCallback((hour: number) => {
    const period = periods.find(item => item.firstHour <= hour && hour <= item.lastHour);
    return period?.exact === false ? exactDay(hour)?.day?.readHour ?? null : period?.readHour ?? null;
  }, [periods, exactDay]);
  const onMenu = useCallback((hour: number, x: number, y: number) => {
    if (track) menu.open(x, y, [jumpItem(runtime, track.id, readAt(hour) ?? hour)]);
  }, [track, readAt, menu.open, runtime]);
  const anchor = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState<{ id: string; name: string } | null>(null);
  const [drillWarnings, setDrillWarnings] = useState<readonly string[]>([]);
  const drillAt = drillHour(drill, clockHour);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...data.temperature === 'available' ? [] : [data.temperature], ...drillWarnings] : null);
  if (!track) return <WidgetState kind="empty" icon="sun" message="Load a campaign to see weather." />;
  if (!window || !extent || extent[1] < extent[0]) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading weather…" />;
  const caption = <div className="weather-caption">
    <div className="air-pattern-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: tokens.surface }} />{show === 'conditions' ? 'No condition' : show === 'temperature' ? 'No average' : 'No modifier'}</span>
      <span><i aria-hidden="true" style={{ background: gap }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />Unknown</span>
    </div>
    {setOption && <Segmented value={show} options={SHOWS} label="Show" onChange={value => setOption('show', value)} />}
  </div>;
  if (!heat || !heat.rows.length) return <div className="air-timeline weather">{caption}<WidgetState kind="empty" message="No regions match these filters." /></div>;
  const chart = <div className="air-timeline-chart" onPointerLeave={() => days.hover(null)}><HeatChart label="Weather by region" rows={heat.rows} periods={periods} labelCatalogue={regions.map(region => region.name)}
      extent={extent} playhead={widget.time.mode === 'follow' && clockHour !== null ? Math.min(clockHour, extent[1]) : null} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={widget.time.mode !== 'follow'} look="split" active={active} revision={`${show}:${modifier}:${data.window.lastHour}`}
      hoursPerSecond={hoursPerSecond} pending={pending} tooltip={tooltip} onContextMenu={onMenu} onSummaryView={onSummaryView}
      pickOwner={track.source.id} onPick={(row, _period, hour) => {
        const region = regions.find(item => item.id === row.id);
        if (!region) return;
        setPicked(region);
        // The provinces of the day under the pointer, whatever its column holds.
        dispatch({ type: 'open', breakdown: 'provinces', hour: pickedDay(hour, dayHours, extent), playing: widget.time.mode === 'follow', playhead: widget.time.mode === 'follow' ? clockHour : null });
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
