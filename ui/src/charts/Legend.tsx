import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { MovingRow } from './rowMotion';
import type { ReactNode } from 'react';
import WindowedRows from './WindowedRows';
import { useChartTokens } from './useChartTokens';
import './charts.css';

export interface LegendItem {
  id: string; label: string; detail?: string; color?: string; tooltip?: string; value?: ReactNode; flag?: ReactNode;
  spokenValue?: string;
  onColor?: (hex: string) => void;
}

export default function Legend({ items, label, always = false, windowed = false, onVisibleItems }: { items: readonly LegendItem[]; label: string; always?: boolean; windowed?: boolean;
  onVisibleItems?: (items: readonly LegendItem[]) => void }) {
  const root = useRef<HTMLUListElement>(null);
  windowed ||= items.length > 30;
  useLayoutEffect(() => () => onVisibleItems?.([]), [onVisibleItems]);
  useEffect(() => {
    if (!windowed || items.length < (always ? 1 : 2)) onVisibleItems?.(items.length >= (always ? 1 : 2) ? items : []);
  }, [items, always, windowed, onVisibleItems]);
  if (items.length < (always ? 1 : 2)) return null;
  if (windowed) return <WindowedLegend items={items} label={label} onVisibleItems={onVisibleItems} />;
  return <ul className="chart-legend" aria-label={label} ref={root}>{items.map(item => <li key={item.id} data-detail={item.detail} data-tooltip={item.tooltip}><Entry item={item} /></li>)}</ul>;
}

function WindowedLegend({ items, label, onVisibleItems }: { items: readonly LegendItem[]; label: string; onVisibleItems?: (items: readonly LegendItem[]) => void }) {
  const rowHeight = useChartTokens().fontSize.small * 2;
  const rows = useMemo(() => items.map(item => ({ key: item.id, item })), [items]);
  const visible = useCallback((rows: readonly MovingRow<LegendItem>[]) => onVisibleItems?.(rows.map(row => row.item)), [onVisibleItems]);
  return <div className="chart-legend-window" style={{ height: Math.min(items.length * rowHeight, rowHeight * 3) }}>
    <WindowedRows items={rows} rowHeight={rowHeight} label={label} className="chart-legend chart-legend-rows" onVisibleRows={visible}>
      {({ item }) => <div className="chart-legend-entry" tabIndex={0} data-detail={item.detail} data-tooltip={item.tooltip}><Entry item={item} /></div>}
    </WindowedRows>
  </div>;
}

function Entry({ item }: { item: LegendItem }) {
  return <>
    {item.onColor
      ? <label className="chart-legend-swatch" style={{ background: item.color }} data-tooltip="Change colour">
        <input type="color" value={item.color && /^#[0-9a-f]{6}$/i.test(item.color) ? item.color : '#888888'} aria-label={`Colour for ${item.label}`}
          onChange={event => item.onColor!(event.currentTarget.value)} /></label>
      : item.color ? <i style={{ background: item.color }} aria-hidden="true" /> : null}
    {item.flag}<span>{item.label}</span>{item.value != null && <b>{item.value}</b>}
    {item.spokenValue != null && <span className="sr-only">: {item.spokenValue}</span>}
  </>;
}
