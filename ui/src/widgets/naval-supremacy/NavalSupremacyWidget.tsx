import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import HeatChart from '../../charts/heat/HeatChart';
import type { HeatRow } from '../../charts/heat/layout';
import { hatchStyle } from '../../charts/hatch';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { useAtHead } from '../../dashboard/atHead';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useProductWindows } from '../../dashboard/productWindows';
import { windowsOf } from '../../dashboard/heldWindows';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { windowExtent } from '../../dashboard/widgetRequests';
import { mix } from '../../shell/appearance/color';
import { followedEnd, mergeSupremacy, runOn, supremacyDays, supremacyRows, supremacyTooltip, unknownReason, validateSupremacy } from './navalSupremacyData';
import type { Control } from './navalSupremacyData';
import { OPTIONS } from './definition';
import './navalSupremacy.css';
import { productRequest } from '../../dashboard/request.ts';

const LABELS: Record<Control, string> = { friendly: 'Friendly control', enemy: 'Enemy control', neither: 'Contested' };

export default function NavalSupremacyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Naval supremacy');
  const following = active && widget.time.mode === 'follow';
  const position = useAtHead(runtime.clock, following, hour => followedEnd(hour, runtime.clock.getSnapshot().playing));
  const window = track && resolveWindow(widget, track, runtime.tracks, following ? position : runtime.clock.getHour());
  const extent: [number, number] | null = track && window ? windowExtent(track, window) : null;
  const moment = extent && extent[0] <= extent[1] ? extent[1] : null;
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const shown = useMemo(() => extent && extent[0] <= extent[1] ? windowsOf(extent[0], extent[1]) : [], [extent?.[0], extent?.[1]]);
  const wanted = useMemo(() => extent && extent[0] <= extent[1] ? windowsOf(extent[0], extent[1], following) : [], [extent?.[0], extent?.[1], following]);
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]) : null;
  const held = useProductWindows(runtime.client, identity, index => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision, product: 'naval-supremacy',
    params: { branchId: track!.branch.id, country: countryFromKey(tag!), window: index } })), wanted, active, validateSupremacy, track?.source);
  const error = shown.map(index => held.errors.get(index)).find(failure => failure !== undefined);
  const retry = held.retry;
  const complete = shown.length > 0 && shown.every(index => held.data.has(index));
  const data = useMemo(() => complete ? mergeSupremacy(new Map(shown.map(index => [index, held.data.get(index)!]))) : null, [complete, shown, held.data]);
  const chosen = contentSet(options.regions).map(contentKey);
  const regions = useMemo(() => data ? data.regions.filter(region => !chosen.length || chosen.includes(region.id)) : [], [data, JSON.stringify(chosen)]);
  const days = useMemo(() => data && extent ? supremacyDays(data, extent) : [], [data, extent?.[0], extent?.[1]]);
  const looks = useCountryLooks(runtime.client, track, tag && moment !== null ? [tag] : [], moment ?? 0, active);
  const colour = useCountryColours(tag ? [tag] : [], looks.color, tokens, looks);
  const viewer = tag && moment !== null ? looks.name(tag, moment) : 'No country selected';
  const colours = useMemo<Record<Control, string>>(() => ({ friendly: colour(tag ?? '', moment ?? undefined), enemy: tokens.error, neither: mix(tokens.surface, tokens.text, 0.2) }),
    [colour, tag, moment, tokens.error, tokens.surface, tokens.text]);
  const rows = useMemo(() => supremacyRows(regions, days, colours, LABELS), [regions, days, colours]);
  const legend = useMemo(() => [{ id: 'friendly', label: `${viewer} and friends`, color: colours.friendly }, { id: 'enemy', label: `Enemies of ${viewer}`, color: colours.enemy },
    { id: 'neither', label: LABELS.neither, color: colours.neither }], [viewer, colours]);
  const dayAt = useCallback((hour: number) => days.find(item => item.firstHour <= hour && hour <= item.lastHour) ?? null, [days]);
  const tooltip = useCallback((row: HeatRow, _period: unknown, hour: number) => {
    const region = regions.find(item => item.id === row.id);
    const item = dayAt(hour);
    if (!data || !region || !item) return null;
    return supremacyTooltip(region, runOn(region.runs, item.index), viewer, LABELS, data.scale, dayLabel(item.lastHour), item.readHour === null ? unknownReason(data, item.index) ?? 'Not recorded' : null);
  }, [data, regions, dayAt, viewer]);
  const onMenu = useCallback((hour: number, x: number, y: number) => { if (track) menu.open(x, y, [jumpItem(runtime, track.id, dayAt(hour)?.lastHour ?? hour)]); }, [track, dayAt, menu.open, runtime]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  useShownEachFrame(runtime.clock, () => data && extent ? liveHead ? Math.min(liveHead.read(), data.lastHour) : extent[1] : null);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see naval supremacy." />;
  if (!tag) return <WidgetState kind="empty" icon="anchor" message="Choose a country in the widget's header." />;
  if (!window || !extent || extent[1] < extent[0]) return <WidgetState kind="outside" />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading naval supremacy…" />;
  if (!rows.length) return <WidgetState kind="empty" message="No sea regions match these filters." />;
  return <div className="supremacy">
    <div className="supremacy-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: `color-mix(in srgb, ${tokens.error} ${tokens.gapOpacity * 100}%, ${tokens.surface})` }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />Unknown</span>
    </div>
    <div className="supremacy-chart"><HeatChart label="Naval supremacy timeline" rows={rows} periods={days} labelCatalogue={regions.map(region => region.name)}
      extent={extent} playhead={widget.time.mode === 'follow' ? extent[1] : null} liveHead={liveHead} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={widget.time.mode !== 'follow'} look="split" active={active} revision={`${tag}:${data.lastHour}:${viewer}`}
      hoursPerSecond={hoursPerSecond} tooltip={tooltip} onContextMenu={onMenu} /></div>
    {menu.menu}
  </div>;
}
