import type { ReactNode } from 'react';
import './charts.css';

export interface LegendItem { id: string; label: string; color: string; value?: string; flag?: ReactNode }

/** A chart's legend: a swatch (or flag) and name per series, with its current value.
 * Always shown for two or more series, so identity never rests on colour alone. */
export default function Legend({ items, label }: { items: readonly LegendItem[]; label: string }) {
  if (items.length < 2) return null;
  return <ul className="chart-legend" aria-label={label}>{items.map(item => <li key={item.id}>
    <i style={{ background: item.color }} aria-hidden="true" />{item.flag}<span>{item.label}</span>{item.value != null && <b>{item.value}</b>}
  </li>)}</ul>;
}
