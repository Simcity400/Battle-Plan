import type { ReactNode } from 'react';
import './charts.css';

export interface LegendItem {
  id: string; label: string; color: string; value?: string; flag?: ReactNode;
  /** A value read out to assistive technology only, where the look shows names alone. */
  spokenValue?: string;
  /** Makes the swatch a colour picker, as faction swatches are; the choice applies campaign-wide. */
  onColor?: (hex: string) => void;
}

/** A chart's legend: a swatch (or flag) and name per series, with its current value.
 * Always shown for two or more series, so identity never rests on colour alone; with
 * `always`, for one too (a diagram whose colours mean something, such as resources). */
export default function Legend({ items, label, always = false }: { items: readonly LegendItem[]; label: string; always?: boolean }) {
  if (items.length < (always ? 1 : 2)) return null;
  return <ul className="chart-legend" aria-label={label}>{items.map(item => <li key={item.id}>
    {item.onColor
      ? <label className="chart-legend-swatch" style={{ background: item.color }} data-tooltip="Change colour">
        <input type="color" value={/^#[0-9a-f]{6}$/i.test(item.color) ? item.color : '#888888'} aria-label={`Colour for ${item.label}`}
          onChange={event => item.onColor!(event.currentTarget.value)} /></label>
      : <i style={{ background: item.color }} aria-hidden="true" />}
    {item.flag}<span>{item.label}</span>{item.value != null && <b>{item.value}</b>}
    {item.spokenValue != null && <span className="sr-only">: {item.spokenValue}</span>}
  </li>)}</ul>;
}
