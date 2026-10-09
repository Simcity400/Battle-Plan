import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import Legend from '../../charts/Legend';
import WindowedRows from '../../charts/WindowedRows';
import { hatchStyle } from '../../charts/hatch';
import { jumpItem } from '../../charts/jump';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { tooltipText } from '../../charts/tooltip';
import { mix, parseColor, readableText } from '../../shell/appearance/color';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { airPercent, airState, airTooltip, inSpans, orderedRegions } from '../../dashboard/airSuperiority';
import { useAir } from '../../dashboard/useAirSuperiority';
import { OPTIONS } from './definition';
import './airSuperiorityRegion.css';

export default function AirSuperiorityRegionWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const air = useAir(props, options, true);
  const menu = useContextMenu('Air superiority region');
  const order = options.order === 'name' ? 'name' : options.order === 'active' ? 'active' : 'contested';
  const rows = useMemo(() => orderedRegions(air.projectedMoment, order).map(row => ({ key: String(row.region.id), item: row })), [air.projectedMoment, order]);
  const tooltip = useCallback((region: number) => air.tooltip(region, air.readHour), [air.tooltip, air.readHour]);
  const onMenu = useCallback((_: number, x: number, y: number) => {
    if (air.track && air.readHour != null) menu.open(x, y, [jumpItem(props.runtime, air.track.id, air.readHour)]);
  }, [air.track, air.readHour, menu, props.runtime]);
  const tips = useRowTooltips({ owner: `air-regions:${props.widget.id}`, active: props.active, content: tooltip,
    revision: `${air.readHour}:${air.hoverRevision}`, onMenu });
  const inactive = mix(air.tokens.surface, air.tokens.text, 0.1);
  const legend = air.legend;
  if (air.needsViewpoint) return <WidgetState kind="empty" icon="plane" message="Choose a dashboard country or a viewpoint in the widget settings." />;
  if (!air.track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see its air superiority." />;
  if (!air.window) return <WidgetState kind="unavailable" message="This time is outside the recorded branch." />;
  if (air.data?.available === false) return <WidgetState kind="unavailable" />;
  if (air.error) return <WidgetState kind="error" message={air.error} onRetry={air.retry} />;
  if (!air.data) return <WidgetState kind="loading" label="Loading air superiority…" />;
  if (order === 'name' && !air.choices) return <WidgetState kind="loading" label="Loading air regions…" />;
  if (air.readHour == null) return <WidgetState kind="unavailable" message="Not recorded at this time." />;
  const knownEmpty = air.data.view === 'moment' && inSpans(air.data.complete, air.readHour) && inSpans(air.data.wingsKnown, air.readHour);
  return <div className="air-regions" data-glide={air.glides} style={air.glideStyle}>
    <Legend label="Air superiority sides" items={legend} always windowed />
    <WindowedRows items={rows} rowHeight={38} fit minRowHeight={16} className="air-region-scroll" label="Air superiority by region"
      empty={<WidgetState kind="empty" message={knownEmpty ? 'No regions have a score or assigned wings at this time.' : 'No active regions could be established at this time.'} />}>
      {({ item: { region, view } }) => <div className="air-region-row" {...tips.rowProps(region.id)}>
        <span id={tips.descriptionId(region.id)} className="sr-only">{tooltipText(airTooltip(region, { ...view, hour: air.readHour }, air.mode.mode === 'viewpoint', air.country, !!air.mode.sides.length, air.factionNames))}</span>
        <span className="air-region-name">{region.name}</span>
        <div className="air-region-bar"
          style={{ backgroundColor: inactive, ...(view.state === 'unknown' ? hatchStyle(air.tokens) : {}) }}>
          {view.state === 'known' ? view.parts.map(part => <span key={part.id} className="air-region-segment"
            style={{ width: `${part.share * 100}%`, background: part.color, color: readableText(parseColor(part.color) ?? parseColor(air.tokens.surface)!) }}>
            <b>{airPercent(part.score, view.total)}</b><i className="air-region-seam" aria-hidden /></span>) : <span className="air-region-state">{airState(view)}</span>}
        </div>
        <span className="air-region-wings">{view.wings == null ? 'Wings unknown' : `${view.wings} ${view.wings === 1 ? 'wing' : 'wings'}`}</span>
      </div>}
    </WindowedRows>
    {menu.menu}
  </div>;
}
