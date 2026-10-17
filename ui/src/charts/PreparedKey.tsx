import { preparedKeyItems } from './prepared';
import type { Span } from './prepared';
import './charts.css';

const LABELS = { approximate: 'Approximate', pending: 'Still being prepared' } as const;

/** The one key line naming a chart's approximate spans (faded, dashed) and
 * the hours still being prepared (a light band); nothing when it has neither. */
export function PreparedKey({ approximate, pending }: { approximate?: readonly Span[]; pending?: readonly Span[] }) {
  const items = preparedKeyItems(approximate, pending);
  if (!items.length) return null;
  return <p className="chart-line-key">{items.map(item => <span key={item}>
    <i className={item === 'approximate' ? 'is-dashed' : 'is-pending'} aria-hidden="true" />{LABELS[item]}
  </span>)}</p>;
}
