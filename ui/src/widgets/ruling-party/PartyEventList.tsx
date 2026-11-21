import type { EventBadge } from '../../charts/badges';
import { dayLabel } from '../../charts/draw';
import { rowJumpProps } from '../../charts/jump';
import WindowedRows from '../../charts/WindowedRows';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { governmentEventRows } from './governmentData';
import type { LedgerGovernment } from './partyTypes';

export default function PartyEventList({ events, total, offset, pageSize, onClose, onPrevious, onNext }: {
  events: readonly EventBadge[]; total: number; offset: number; pageSize: number; onClose: () => void; onPrevious: () => void; onNext: () => void;
}) {
  return <div className="ruling-party-events" aria-label="Party events">
    <div className="chart-line-key"><strong>{total === 1 ? '1 event' : `${total} events`}</strong>
      <button type="button" className="button small" onClick={onClose}>Close events</button></div>
    <ul>{events.map((badge, index) => <li key={index}>{dayLabel(badge.hour)}: {badge.glyph} {badge.label}</li>)}</ul>
    {total > pageSize && <div className="chart-line-key">
      <button type="button" className="button small" disabled={offset === 0} onClick={onPrevious}>Previous events</button>
      <span>{offset + 1}-{Math.min(total, offset + pageSize)} of {total}</span>
      <button type="button" className="button small" disabled={offset + pageSize >= total} onClick={onNext}>Next events</button>
    </div>}
  </div>;
}

export function GovernmentChangeList({ rows, name, runtime, trackId, openMenu }: {
  rows: readonly LedgerGovernment[]; name: (token: string) => string; runtime: WidgetRuntime; trackId: string;
  openMenu: Parameters<typeof rowJumpProps>[3];
}) {
  return <section className="ruling-party-changes" aria-label="Government changes">
    <div className="chart-line-key"><strong>Government changes</strong><span>{rows.length} recorded changes</span></div>
    <WindowedRows label="Recorded government changes" items={rows.map(row => ({ key: JSON.stringify([row.country, row.instance, row.epoch, row.sequence]), item: row }))} rowHeight={64}>
      {row => <div tabIndex={0} {...rowJumpProps(runtime, trackId, row.item.hour, openMenu)}>{governmentEventRows(row.item, name)
        .map((detail, index) => <div key={index}>{detail.label}{detail.value ? `: ${detail.value}` : ''}</div>)}</div>}
    </WindowedRows>
  </section>;
}
