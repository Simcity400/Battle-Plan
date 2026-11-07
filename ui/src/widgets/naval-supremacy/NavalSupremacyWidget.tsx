import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import HeatChart from '../../charts/heat/HeatChart';
import { hatchStyle } from '../../charts/hatch';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { NAME_LOADING, useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { keyedDays } from '../../dashboard/keyedDays';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { mix } from '../../shell/appearance/color';
import { regionDay, regionSeries, supremacyRows, supremacyTooltip } from './navalSupremacyData';
import type { Control } from './navalSupremacyData';
import { useNavalSupremacy } from './useNavalSupremacy';
import { windowExtent } from '../../dashboard/widgetRequests';
import '../air-superiority-timeline/airSuperiorityTimeline.css';

const LABELS: Record<Control, string> = { friendly: 'Friendly control', enemy: 'Enemy control', neither: 'Contested' };
const pendingCountry = () => ({ name: NAME_LOADING });

/** Each sea region's naval control day by day from one country's point of view, in the
 * air superiority timeline's form, by the game's own control rule. */
export default function NavalSupremacyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playback = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Naval supremacy');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const extent: [number, number] | null = track && window ? windowExtent(track, window) : null;
  // The moment shown, in this campaign's hours: the viewer's name and colour are as then.
  const moment = extent && extent[0] <= extent[1] ? extent[1] : null;
  const { tag, options, data, error, retry } = useNavalSupremacy(widget, runtime, active, extent);
  const chosen = contentSet(options.regions).map(contentKey);
  const regions = useMemo(() => data ? data.regions.filter(region => !chosen.length || chosen.includes(region.id)) : [], [data, JSON.stringify(chosen)]); // eslint-disable-line react-hooks/exhaustive-deps
  // A reply kept while a moved window loads answers only its own hours: the rest are loading, not unrecorded.
  const covered = useMemo<[number, number] | null>(() => data && extent
    ? [Math.max(extent[0], data.window.firstHour), Math.min(extent[1], data.window.lastHour)] : null, [data, extent?.[0], extent?.[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const days = useMemo(() => data && covered && covered[0] <= covered[1] ? keyedDays(data.window, covered, data.dayHours) : [], [data, covered]);
  const loading = useMemo(() => {
    if (!extent || !covered) return undefined;
    if (covered[0] > covered[1]) return [{ firstHour: extent[0], lastHour: extent[1] }];
    const spans = [...covered[0] > extent[0] ? [{ firstHour: extent[0], lastHour: covered[0] - 1 }] : [],
      ...covered[1] < extent[1] ? [{ firstHour: covered[1] + 1, lastHour: extent[1] }] : []];
    return spans.length ? spans : undefined;
  }, [extent?.[0], extent?.[1], covered]); // eslint-disable-line react-hooks/exhaustive-deps
  const looks = useCountryLooks(runtime.client, track, tag && moment !== null ? [tag] : [], moment ?? 0, active);
  const colour = useCountryColours(tag ? [tag] : [], looks.color, tokens, looks);
  const viewer = tag && moment !== null ? looks.name(tag, moment) : 'No country selected';
  const colours = useMemo<Record<Control, string>>(() => ({ friendly: colour(tag ?? '', moment ?? undefined), enemy: tokens.error, neither: mix(tokens.surface, tokens.text, 0.2) }),
    [colour, tag, moment, tokens.error, tokens.surface, tokens.text]);
  // Each day's control is from whoever held the viewer's tag at its read: named as then.
  const hover = useHoverLabels(runtime.client, track, active, pendingCountry);
  const rows = useMemo(() => data ? supremacyRows(data, regions, days, colours, LABELS) : [], [data, regions, days, colours]);
  const series = useMemo(() => data ? regionSeries(data.window) : new Map(), [data]);
  const legend = useMemo(() => [{ id: 'friendly', label: `${viewer} and friends`, color: colours.friendly }, { id: 'enemy', label: `Enemies of ${viewer}`, color: colours.enemy },
    { id: 'neither', label: LABELS.neither, color: colours.neither }], [viewer, colours]);
  const dayAt = useCallback((hour: number) => days.find(day => day.firstHour <= hour && hour <= day.lastHour) ?? null, [days]);
  const { ask, labelAt } = hover;
  const tooltip = useCallback((row: { id: string }, _period: unknown, hour: number) => {
    const region = regions.find(item => item.id === row.id);
    if (!data || !region) return null;
    if (loading?.some(span => span.firstHour <= hour && hour <= span.lastHour)) return { title: region.name, rows: [{ label: dayLabel(Math.floor(hour)) }, { label: 'Loading…' }] };
    const day = dayAt(hour);
    if (day?.readHour == null || !tag) return supremacyTooltip(region, null, viewer, LABELS, data.scale, dayLabel(Math.floor(hour)), null);
    ask(day.readHour, [tag]);
    return { ...supremacyTooltip(region, regionDay(series.get(region.id), day.at), labelAt(day.readHour)(tag).name, LABELS, data.scale,
      dayLabel(Math.floor(hour)), dayLabel(day.readHour)), ...countryNameDetails(labelAt(day.readHour)(tag)) };
  }, [data, regions, dayAt, series, viewer, tag, ask, labelAt, loading]);
  const onMenu = useCallback((hour: number, x: number, y: number) => {
    if (track) menu.open(x, y, [jumpItem(runtime, track.id, dayAt(hour)?.readHour ?? hour)]);
  }, [track, dayAt, menu.open, runtime]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see naval supremacy." />;
  if (!tag) return <WidgetState kind="empty" icon="anchor" message="Choose a country in the widget's header." />;
  if (!window || !extent || extent[1] < extent[0]) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading naval supremacy…" />;
  if (!rows.length) return <WidgetState kind="empty" message="No sea regions match these filters." />;
  const prefix = preparedPrefix(data);
  return <div className="air-timeline">
    <div className="air-pattern-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: `color-mix(in srgb, ${tokens.error} ${tokens.gapOpacity * 100}%, ${tokens.surface})` }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />Unknown</span>
    </div>
    <div className="air-timeline-chart"><HeatChart label="Naval supremacy timeline" rows={rows} periods={days} labelCatalogue={regions.map(region => region.name)}
      extent={extent} playhead={widget.time.mode === 'follow' ? extent[1] : null} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={widget.time.mode !== 'follow'} look="split" active={active} revision={`${tag}:${data.window.lastHour}:${viewer}:${hover.revision}`}
      hoursPerSecond={hoursPerSecond} pending={prefix?.pending} loading={loading} tooltip={tooltip} onContextMenu={onMenu} /></div>
    {menu.menu}
  </div>;
}
