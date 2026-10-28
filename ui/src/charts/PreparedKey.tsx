import { preparedKeyItems } from './prepared';
import type { Span } from './prepared';
import './charts.css';

const LABELS = { approximate: 'Approximate', pending: 'Still being prepared', loading: 'Loading' } as const;

/** The one key line naming a chart's approximate spans (faded, dashed), the hours still
 * being prepared and the hours still loading (a light band); nothing when it has none. */
export function PreparedKey({ approximate, pending, loading }: { approximate?: readonly Span[]; pending?: readonly Span[]; loading?: readonly Span[] }) {
  const items = preparedKeyItems(approximate, pending, loading);
  if (!items.length) return null;
  return <p className="chart-line-key">{items.map(item => <span key={item}>
    <i className={item === 'approximate' ? 'is-dashed' : 'is-pending'} aria-hidden="true" />{LABELS[item]}
  </span>)}</p>;
}
