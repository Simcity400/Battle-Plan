import { useMemo } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { Segmented } from '../../components/Segmented';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { formatAmount, formatRate, formatSigned } from '../../charts/valueAxis';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { WidgetProps } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { readAt } from '../../dashboard/reads';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import { AI_NOTE, balanceMode, formatPercent, groupTooltip, stockGroups } from './stockpileData';
import type { StockGroup } from './stockpileData';
import type { StockpileMode } from './stockpileDrill';
import { stockRows } from './stockpileView';
import type { StockCount, StockRow } from './stockpileView';
import { useStockpileArtwork } from './useStockpileArtwork';
import { approximateAt, preparedPrefix } from '../../dashboard/preparedPrefix';

/** Two lines a row: the type, and its models. */
const ROW_HEIGHT = 40;

const COUNTS: readonly { value: StockCount; label: string; tooltip: string }[] = [
  { value: 'units', label: 'Units', tooltip: 'Output in units a day' },
  { value: 'ic', label: 'IC', tooltip: 'Output as the IC its lines put in a day; stock and surplus have no IC value' },
];

/** One country's stockpile at an hour: every equipment type with anything then, led by
 * the game's surplus or shortage (shortages first), with its models and output a day.
 * For an AI-controlled country, for which the game computes none, what it holds. Rows
 * open the type's breakdown; the rest of the types are counted, not listed. */
export default function StockpileTable({ data, hour, included, count, setCount, active, runtime, track, viewport, onDiagnostics, onPick }: {
  data: EquipmentStockpileReply; hour: number; included: readonly string[]; count: StockCount; setCount?: (count: StockCount) => void;
  active: boolean; runtime: WidgetProps['runtime']; track: BoundTrack; viewport: number; onDiagnostics: WidgetProps['onDiagnostics'];
  onPick: (group: StockGroup, hour: number, mode: StockpileMode) => void;
}) {
  const mode = balanceMode(data, hour);
  const player = mode === 'player';
  const groups = useMemo(() => stockGroups(data, 'type', hour, included), [data, hour, included]);
  const { rows, quiet } = useMemo(() => stockRows(data, groups, hour, mode, count), [data, groups, hour, mode, count]);
  const artCoverage = player ? data.balanceHours : data.stockHours;
  const artHour = runAt(artCoverage, hour) ? readAt(player ? data.logisticsReads : data.reads, hour) : null;
  const artwork = useStockpileArtwork(runtime, track, data.tag, artHour, [], active);
  const art = useMemo(() => new Map(artwork?.data?.entries.map(entry => [entry.key, entry]) ?? []), [artwork]);
  const diagnostics = useMemo(() => [...data.warnings, ...(artwork?.data?.warnings ?? []), ...(artwork?.error ? [artwork.error] : [])], [data, artwork]);
  useDiagnostics(onDiagnostics, diagnostics);
  const read = player ? readAt(data.logisticsReads, hour) : readAt(data.reads, hour);
  const columns = useMemo<TableColumn<StockRow>[]>(() => [
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.group.label,
      render: row => {
        const picture = art.get(`type:${row.group.key}`);
        return <span className="stockpile-equipment">
          {picture?.icon ? <img className="stockpile-art" src={picture.icon} alt="" /> : <span className="stockpile-art"><ArtFallback icon="hammer" /></span>}
          <span><b>{row.group.label}</b>{row.models && <small>{row.models}</small>}</span>
        </span>;
      } },
    player
      ? { key: 'surplus', header: 'Surplus / shortage', label: 'Surplus or shortage', numeric: true, width: 'max-content', value: row => row.surplus,
        hint: 'The game’s Logistics figure: available minus needed', renderMissing: true,
        render: row => row.surplus == null ? <span className="stockpile-missing">Not known</span>
          : <span className={`stockpile-balance${row.surplus < 0n ? ' is-negative' : row.surplus > 0n ? ' is-positive' : ''}`}>
            {formatSigned(row.surplus, data.decimals, 0)}{row.share != null && <small>{formatPercent(row.share)} of need</small>}
          </span> }
      : { key: 'stock', header: 'In stock', label: 'In stock', numeric: true, width: 'max-content', value: row => row.stock, renderMissing: true,
        render: row => row.stock == null ? <span className="stockpile-missing">Not recorded</span> : formatAmount(row.stock, data.decimals, 0) },
    { key: 'output', header: count === 'ic' ? 'IC / day' : 'Output / day', label: count === 'ic' ? 'IC a day' : 'Output a day', numeric: true, width: 'max-content',
      value: row => row.output, renderMissing: true,
      render: row => row.output == null ? <span className="stockpile-missing">Not recorded</span>
        : <span className={row.output === 0n ? 'stockpile-zero-output' : undefined}>{formatRate(row.output, data.decimals)}</span> },
  ], [art, player, count, data.decimals]);
  const date = dayLabel(hour);
  return <div className="stockpile-table">
    {approximateAt(preparedPrefix(data), hour) && <p className="chart-line-key">Approximate equipment types while preparation continues.</p>}
    <div className="stockpile-caption-line">
      <span>On {date} · {player ? 'surplus or shortage from the game’s Logistics screen' : mode === 'ai' ? AI_NOTE : 'in stock'}</span>
      {setCount && <Segmented value={count} options={COUNTS} label="Count" onChange={setCount} />}
    </div>
    {rows.length === 0 ? <p className="stockpile-empty">Nothing of the included types was recorded on {date}.</p>
      : <div className="stockpile-table-rows" style={{ height: Math.min(rows.length * ROW_HEIGHT + 28, Math.max(160, viewport)) }}>
        <Table label={`Equipment stockpile on ${date}`} rows={rows} columns={columns} rowKey={row => row.group.key} rowHeight={ROW_HEIGHT}
          sort={player ? { column: 'surplus', direction: 'ascending' } : { column: 'stock', direction: 'descending' }}
          onRowClick={row => onPick(row.group, hour, player ? 'balance' : 'stock')}
          tooltip={row => groupTooltip(data, row.group, 'type', hour, { title: row.group.label, subtitle: date,
            note: art.get(`type:${row.group.key}`)?.name ?? undefined }, viewport)}
          menu={() => read == null ? [] : [jumpItem(runtime, track.id, read)]} />
      </div>}
    {quiet > 0 && <p className="stockpile-empty">{quiet.toLocaleString()} more {quiet === 1 ? 'type' : 'types'}: nothing in stock, being made or needed on {date}.</p>}
  </div>;
}
