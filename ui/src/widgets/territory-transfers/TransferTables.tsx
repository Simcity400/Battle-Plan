import { useMemo } from 'react';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import type { Sort } from '../../charts/table/logic';
import type { NarrowTable } from '../../charts/table/options';
import type { TooltipContent } from '../../charts/tooltip';
import { usNumber } from '../../charts/numberFormat';
import { formatSigned } from '../../charts/valueAxis';
import type { MenuItem } from '../../components/ContextMenu';
import type { WidgetProps } from '../../dashboard/definitions';
import { netVictoryPoints, transferDate } from './territoryData';
import type { CountryBalance, TransferOccurrence, TransferSide } from './territoryData';
import type { TransferLabels } from './territoryTooltip';

/** A signed whole number in the positive or error colour, as balances are drawn elsewhere. */
function Signed({ value }: { value: bigint }) {
  return <span className={value > 0n ? 'transfer-gain' : value < 0n ? 'transfer-loss' : undefined}>{formatSigned(value, 0, 0)}</span>;
}

const countryKey = (row: CountryBalance) => row.key;
const transferKey = (row: TransferOccurrence) => row.key;

interface Shared {
  labels: TransferLabels; sort: Sort; setOption?: WidgetProps['setOption'];
  cellBars: boolean; density: 'comfortable' | 'compact'; hoursPerSecond: number; narrow: NarrowTable;
}

const countryCell = (labels: TransferLabels, side: TransferSide, quiet = false, suffix?: string) => {
  const label = labels.country(side);
  return <span className={quiet ? 'transfer-country is-quiet' : 'transfer-country'}><CountryCell name={label.name} flag={label.flag} flagPending={label.flagPending} flagReason={label.flagReason} />
    {suffix && <small>{suffix}</small>}</span>;
};

export function CountryTable({ rows, labels, sort, setOption, cellBars, density, narrow, hoursPerSecond, onPick, tooltip, menu, onVisibleRows }: Shared & {
  rows: readonly CountryBalance[]; onPick: (row: CountryBalance) => void; tooltip: (row: CountryBalance) => TooltipContent;
  menu: (row: CountryBalance) => MenuItem[]; onVisibleRows: (rows: readonly CountryBalance[]) => void;
}) {
  const columns = useMemo<TableColumn<CountryBalance>[]>(() => [
    { key: 'country', header: 'Country', label: 'Country', value: row => labels.country(row.side).name, render: row => countryCell(labels, row.side),
      width: 'minmax(var(--table-name-min), 1fr)' },
    { key: 'gained', header: 'Gained', label: 'Gained', numeric: true, bar: 'visible', value: row => row.gained, render: row => usNumber.format(row.gained), width: 'max-content' },
    { key: 'lost', header: 'Lost', label: 'Lost', numeric: true, bar: 'visible', value: row => row.lost, render: row => usNumber.format(row.lost), width: 'max-content' },
    { key: 'net', header: 'Net', label: 'Net', numeric: true, priority: 2, text: row => formatSigned(BigInt(row.gained - row.lost), 0, 0), value: row => row.gained - row.lost, render: row => <Signed value={BigInt(row.gained - row.lost)} />, width: 'max-content' },
    { key: 'netVictoryPoints', header: 'Net VP', label: 'Net VP', priority: 1, text: row => { const value = netVictoryPoints(row); return value == null ? null : formatSigned(value, 0, 0); }, hint: 'Victory points gained minus lost; unknown when any of the states’ points were not recorded.',
      numeric: true, value: netVictoryPoints, render: row => <Signed value={netVictoryPoints(row)!} />, width: 'max-content' },
  ], [labels]);
  return <Table label="Territory transfers by country" rows={rows} columns={columns} rowKey={countryKey} sort={sort} setOption={setOption}
    cellBars={cellBars} density={density} narrow={narrow} hoursPerSecond={hoursPerSecond} onRowClick={onPick} tooltip={tooltip} menu={menu} onVisibleRows={onVisibleRows} />;
}

export function TransferTable({ rows, labels, sort, setOption, cellBars, density, narrow, hoursPerSecond, focus, tooltip, menu, onVisibleRows, label }: Shared & {
  rows: readonly TransferOccurrence[];
  /** The drill's country: dimmed where it appears, and VP signed from its side. */
  focus?: string | null;
  tooltip: (row: TransferOccurrence) => TooltipContent; menu: (row: TransferOccurrence) => MenuItem[];
  onVisibleRows: (rows: readonly TransferOccurrence[]) => void; label: string;
}) {
  const columns = useMemo<TableColumn<TransferOccurrence>[]>(() => {
    const signedVictoryPoints = (row: TransferOccurrence) => row.victoryPoints == null || focus == null ? row.victoryPoints
      : row.to.key === focus ? row.victoryPoints : -row.victoryPoints;
    return [
      { key: 'state', header: 'State', label: 'State', value: row => labels.state(row), width: 'minmax(7em, 1fr)' },
      { key: 'from', header: 'From', label: 'From', value: row => row.from ? labels.country(row.from).name : null, renderMissing: true,
        render: row => row.from ? countryCell(labels, row.from, row.from.key === focus) : <span className="transfer-unknown">Not recorded</span>,
        width: 'minmax(var(--table-name-min), 1.2fr)' },
      { key: 'to', header: 'To', label: 'To', value: row => labels.country(row.to).name,
        render: row => countryCell(labels, row.to, row.to.key === focus, row.released ? ' · released' : undefined), width: 'minmax(var(--table-name-min), 1.2fr)' },
      { key: 'date', header: 'Date', label: 'Date', value: row => row.afterHour, render: row => transferDate(row.afterHour), width: 'max-content', priority: 1 },
      { key: 'victoryPoints', header: 'VP', label: 'VP', hint: 'Victory points', numeric: true, value: signedVictoryPoints, priority: 2,
        text: row => { const value = signedVictoryPoints(row); return value == null ? null : focus == null ? usNumber.format(value) : formatSigned(BigInt(value), 0, 0); },
        render: row => focus == null ? usNumber.format(row.victoryPoints!) : <Signed value={signedVictoryPoints(row)!} />, width: 'max-content' },
    ];
  }, [labels, focus]);
  return <Table label={label} rows={rows} columns={columns} rowKey={transferKey} sort={sort} setOption={setOption}
    cellBars={cellBars} density={density} narrow={narrow} hoursPerSecond={hoursPerSecond} tooltip={tooltip} menu={menu} onVisibleRows={onVisibleRows} />;
}
