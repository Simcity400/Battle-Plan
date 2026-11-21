import type { ReactNode } from 'react';
import WidgetState from '../../components/WidgetState';
import type { Failure } from '../../dashboard/queryError';
import { TooltipRowView } from '../TooltipRows';
import type { TooltipRow } from '../tooltip';
import './countryRace.css';

export default function MeasureDetails({ title, rows, error, onRetry, onClose, children }: {
  title: string; rows: readonly TooltipRow[] | undefined; error?: Failure; onRetry?: () => void; onClose?: () => void; children?: ReactNode;
}) {
  return <section className="parts-race-details" aria-label={title}>
    <header className="chart-tooltip-row is-strong"><span>{title}</span>{onClose && <button type="button" className="button is-quiet" onClick={onClose}>Close</button>}</header>
    {error ? <WidgetState kind="error" failure={error} onRetry={onRetry} />
      : !rows ? <WidgetState kind="loading" label="Loading…" />
        : <div className="drill-parts">{rows.map((row, index) => <TooltipRowView key={`${index}:${row.label}`} row={row} />)}</div>}
    {children}
  </section>;
}
