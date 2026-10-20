import { useMemo } from 'react';
import type { NarrowTable } from '../../charts/table/options';
import { CountryCell } from '../../charts/table/Table';
import { usNumber } from '../../charts/numberFormat';
import { formatSigned } from '../../charts/valueAxis';
import type { TooltipContent } from '../../charts/tooltip';
import type { MenuItem } from '../../components/ContextMenu';
import type { Sort } from '../../charts/table/logic';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryBalances, netVictoryPoints } from './territoryData';
import type { TransferOccurrence } from './territoryData';
import type { TransferLabels } from './territoryTooltip';
import { TransferTable } from './TransferTables';

const states = (count: number) => `${count === 1 ? 'state' : 'states'}`;

/** One country's transfers at the drill's day: what it gained and lost, then every state. */
export default function TransferDrill({ rows, focus, labels, named, outside, ...table }: {
  rows: readonly TransferOccurrence[]; focus: string | null; labels: TransferLabels;
  /** A pinned breakdown names its country; the drill's header already does. */
  named: boolean; outside: boolean;
  sort: Sort; setOption?: WidgetProps['setOption']; cellBars: boolean; density: 'comfortable' | 'compact'; narrow: NarrowTable; hoursPerSecond: number;
  tooltip: (row: TransferOccurrence) => TooltipContent; menu: (row: TransferOccurrence) => MenuItem[];
  onVisibleRows: (rows: readonly TransferOccurrence[]) => void;
}) {
  const balance = useMemo(() => focus == null ? null : countryBalances(rows).find(row => row.key === focus) ?? null, [rows, focus]);
  if (outside) return <p className="drill-empty">This time is outside the selected recorded interval.</p>;
  const net = balance && netVictoryPoints(balance);
  const label = balance && labels.country(balance.side);
  return <div className="transfer-drill">
    {balance && <p className="transfer-caption">
      {named && label && <span className="transfer-caption-country"><CountryCell name={label.name} flag={label.flag} flagPending={label.flagPending} flagReason={label.flagReason} /></span>}
      Gained <b>{usNumber.format(balance.gained)}</b> {states(balance.gained)} · lost <b>{usNumber.format(balance.lost)}</b>
      {net != null && <> · net VP <b className={net > 0n ? 'transfer-gain' : net < 0n ? 'transfer-loss' : undefined}>{formatSigned(net, 0, 0)}</b></>}
    </p>}
    {rows.length ? <TransferTable {...table} rows={rows} labels={labels} focus={focus} label="Country territory transfers" />
      : <p className="drill-empty">{focus == null ? 'No territory transfers yet.' : 'No territory transfers for this country at this date.'}</p>}
  </div>;
}
