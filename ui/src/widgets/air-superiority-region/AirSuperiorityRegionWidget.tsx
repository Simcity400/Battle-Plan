import { PreparedKey } from '../../charts/PreparedKey';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useParityRows } from '../../dashboard/parityRows';
import { useCallback, useMemo, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
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

/** Pixels a segment must exceed to show its share, and to be set off from the next by a seam. */
const LABEL_MIN = 34, SEAM_MIN = 3;

export default function AirSuperiorityRegionWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const air = useAir(props, options, true);
  const menu = useContextMenu('Air superiority region');
  const order = options.order === 'name' ? 'name' : options.order === 'active' ? 'active' : 'contested';
  const rows = useMemo(() => orderedRegions(air.projectedMoment, order).map(row => ({ key: String(row.region.id), item: row })), [air.projectedMoment, order]);
  const tooltip = useCallback((region: number) => air.tooltip(region, air.readHour), [air.tooltip, air.readHour]);
  const onMenu = useCallback((_: number, x: number, y: number) => {
    if (air.track && air.readHour != null) menu.open(x, y, [jumpItem(props.runtime, air.track.id, air.readHour)]);
  }, [air.track, air.readHour, menu.open, props.runtime]);
  const tips = useRowTooltips({ owner: `air-regions:${props.widget.id}`, active: props.active, content: tooltip,
    revision: `${air.readHour}:${air.hoverRevision}`, onMenu });
  const inactive = mix(air.tokens.surface, air.tokens.text, 0.1);
  const textOn = useMemo(() => {
    const known = new Map<string, string>();
    return (colour: string) => {
      let text = known.get(colour);
      if (text === undefined) { text = readableText(parseColor(colour) ?? parseColor(air.tokens.surface)!); known.set(colour, text); }
      return text;
    };
  }, [air.tokens.surface]);
  // One observer measures every bar, before paint, so labels and seams follow each share
  // without a container query evaluated on every frame of a gliding width.
  const [widths, setWidths] = useState<ReadonlyMap<number, number>>(new Map());
  const measure = useMemo(() => {
    if (typeof ResizeObserver === 'undefined') return undefined;
    const observer = new ResizeObserver(entries => flushSync(() => setWidths(before => {
      const next = new Map(before);
      for (const { target, contentRect } of entries) next.set(Number((target as HTMLElement).dataset.region), contentRect.width);
      return next;
    })));
    return (element: HTMLDivElement | null) => {
      if (!element) return;
      observer.observe(element);
      return () => observer.unobserve(element);
    };
  }, []);
  const legend = air.legend;
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: 'Air superiority region values', complete: !!air.data,
    hour: air.hour, total: air.data?.view === 'moment' ? air.data.regions.length : 0,
    rows: air.data?.view === 'moment' ? air.data.regions : [] }));
  if (air.needsViewpoint) return <WidgetState kind="empty" icon="plane" message="Choose a dashboard country or a viewpoint in the widget settings." />;
  if (!air.track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see its air superiority." />;
  if (!air.window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (pendingReplyAt(air.data, air.hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (air.data?.available === false) return <WidgetState kind="unavailable" />;
  if (air.error) return <WidgetState kind="error" failure={air.error} onRetry={air.retry} />;
  if (!air.data) return <WidgetState kind="loading" label="Loading air superiority…" />;
  if (order === 'name' && !air.choices) return <WidgetState kind="loading" label="Loading air regions…" />;
  if (air.readHour == null) return <WidgetState kind="unavailable" message="Not recorded at this time." />;
  const knownEmpty = air.data.view === 'moment' && inSpans(air.data.complete, air.readHour) && inSpans(air.data.wingsKnown, air.readHour);
  return <div ref={root} className="air-regions" data-glide={air.glides} style={air.glideStyle}>
    <PreparedKey pending={preparedPrefix(air.data)?.pending} approximate={preparedPrefix(air.data)?.approximate} />
    <Legend label="Air superiority sides" items={legend} always windowed />
    <WindowedRows items={rows} rowHeight={38} fit minRowHeight={16} className="air-region-scroll" label="Air superiority by region"
      empty={<WidgetState kind="empty" message={knownEmpty ? 'No regions have a score or assigned wings at this time.' : 'Air activity is unknown at this time.'} />}>
      {({ item: { region, view } }) => {
        const width = widths.get(region.id);
        return <div className="air-region-row" {...tips.rowProps(region.id)}>
          {/* Only the row hovered or focused, which its description is read for, is described. */}
          {tips.hovered === region.id && <span id={tips.descriptionId(region.id)} className="sr-only">{tooltipText(airTooltip(region, { ...view, hour: air.readHour }, air.mode.mode === 'viewpoint', air.country, !!air.mode.sides.length, air.factionNames))}</span>}
          <span className="air-region-name">{region.name}</span>
          <div className="air-region-bar" ref={measure} data-region={region.id}
            style={{ backgroundColor: inactive, ...(view.state === 'unknown' ? hatchStyle(air.tokens) : {}) }}>
            {view.state === 'known' ? view.parts.map((part, index) => <span key={part.id} className="air-region-segment"
              style={{ width: `${part.share * 100}%`, background: part.color, color: textOn(part.color) }}>
              {(width === undefined || part.share * width > LABEL_MIN) && <b>{airPercent(part.score, view.total)}</b>}
              {index < view.parts.length - 1 && (width === undefined || part.share * width > SEAM_MIN) && <i className="air-region-seam" aria-hidden />}</span>) : <span className="air-region-state">{airState(view)}</span>}
          </div>
          <span className="air-region-wings">{view.wings == null ? 'Wings unknown' : `${view.wings} ${view.wings === 1 ? 'wing' : 'wings'}`}</span>
        </div>;
      }}
    </WindowedRows>
    {menu.menu}
  </div>;
}
