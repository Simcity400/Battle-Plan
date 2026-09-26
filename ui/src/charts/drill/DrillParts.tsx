import './drill.css';

export interface DrillPart { id: string; label: string; value: number; color?: string }

/** The parts of a value at one moment, largest first, each with its amount and its
 * share of the total. Labels come from the game or the recording only; whatever the
 * recording cannot attribute is shown as its own row, never folded into another. */
export default function DrillParts({ parts, format, total, unattributed }: {
  parts: readonly DrillPart[]; format: (value: number) => string; total?: number;
  /** The part of the total no recorded source explains, with the game's own label if it has one. */
  unattributed?: { label: string; value: number };
}) {
  const rows = [...parts].sort((a, b) => Math.abs(b.value) - Math.abs(a.value) || a.label.localeCompare(b.label));
  if (unattributed && unattributed.value !== 0) rows.push({ id: '__unattributed', label: unattributed.label, value: unattributed.value });
  const widest = Math.max(1e-12, ...rows.map(row => Math.abs(row.value)));
  const sum = total ?? rows.reduce((result, row) => result + row.value, 0);
  const share = (value: number) => sum ? `${Math.round(Math.abs(value) / Math.abs(sum) * 1000) / 10}%` : '—';
  if (!rows.length) return <p className="drill-empty">Nothing recorded for this day.</p>;
  return <ol className="drill-parts">{rows.map(row => <li key={row.id} className={row.id === '__unattributed' ? 'is-unattributed' : undefined}>
    <span className="drill-part-label">{row.label}</span>
    <span className="drill-part-bar" aria-hidden="true"><i style={{ width: `${Math.abs(row.value) / widest * 100}%`,
      background: row.color ?? (row.value < 0 ? 'var(--error)' : 'var(--positive)') }} /></span>
    <b className={row.value < 0 ? 'is-negative' : undefined}>{format(row.value)}</b>
    <small>{share(row.value)}</small>
  </li>)}</ol>;
}
