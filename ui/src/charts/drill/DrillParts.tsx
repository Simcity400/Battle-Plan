import './drill.css';

/** One part: its value (for the bar and share), an exact text for it if the widget
 * formats one, and its colour in the widget's chart (shown as a swatch too). */
export interface DrillPart { id: string; label: string; value: number; color?: string; text?: string;
  /** The label is the game's own text rather than an identified source: shown muted. */
  muted?: boolean }

/** The parts of a value at one moment, largest first, each with its amount and its
 * share of the total. Labels come from the game or the recording only; whatever the
 * recording cannot attribute is shown as its own row, never folded into another. */
export default function DrillParts({ parts, format, total, unattributed, totalRow }: {
  parts: readonly DrillPart[]; format: (value: number) => string; total?: number;
  /** The part of the total no recorded source explains, with the game's own label if it has one. */
  unattributed?: { label: string; value: number; text?: string; color?: string };
  /** A closing total row, as signed off (mock-up drill-1). */
  totalRow?: { label: string; value: string };
}) {
  const rows = [...parts].sort((a, b) => Math.abs(b.value) - Math.abs(a.value) || a.label.localeCompare(b.label));
  if (unattributed && unattributed.value !== 0) rows.push({ id: '__unattributed', label: unattributed.label, value: unattributed.value, text: unattributed.text, color: unattributed.color });
  const widest = Math.max(1e-12, ...rows.map(row => Math.abs(row.value)));
  const sum = total ?? rows.reduce((result, row) => result + row.value, 0);
  const share = (value: number) => sum ? `${Math.round(Math.abs(value) / Math.abs(sum) * 1000) / 10}%` : '—';
  if (!rows.length && !totalRow) return <p className="drill-empty">Nothing recorded for this day.</p>;
  return <ol className="drill-parts">{rows.map(row => <li key={row.id} className={row.id === '__unattributed' ? 'is-unattributed' : row.muted ? 'is-muted' : undefined}>
    <span className="drill-part-label">{row.color && <i className="drill-part-swatch" aria-hidden="true" style={{ background: row.color }} />}{row.label}</span>
    <span className="drill-part-bar" aria-hidden="true"><i style={{ width: `${Math.abs(row.value) / widest * 100}%`,
      background: row.color ?? (row.value < 0 ? 'var(--error)' : 'var(--positive)') }} /></span>
    <b className={row.value < 0 ? 'is-negative' : undefined}>{row.text ?? format(row.value)}</b>
    <small>{share(row.value)}</small>
  </li>)}{totalRow && <li className="is-total"><span className="drill-part-label">{totalRow.label}</span><span /><b>{totalRow.value}</b><small /></li>}</ol>;
}
