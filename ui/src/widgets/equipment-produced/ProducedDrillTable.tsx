import { useMemo } from 'react';
import ArtFallback from '../../components/ArtFallback';
import Legend from '../../charts/Legend';
import { dayLabel } from '../../charts/draw';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { readAt } from '../../dashboard/reads';
import type { EquipmentProducedReply } from '../../generated/dashboard';
import { useStockpileArtwork } from '../equipment-stockpile/useStockpileArtwork';
import { countedSince, formatRaw, producedAt, producedModel, producedRequest, requestParams, validateProducedReply } from './producedData';
import type { Measure } from './producedData';
import { countDecimals, producedWindow, producedWindowUnavailable, typeRows } from './producedView';
import type { Count, TypeRow } from './producedView';

/** Two lines a row: the type, and its models. */
const ROW_HEIGHT = 40;

/** Every type a country made in the window shown, with its models: units delivered
 * and IC put in. Only IC is one currency, so only it has bars, shares and a total;
 * units of different types are never added up. A row picks the type the chart below
 * stacks by model. */
export default function ProducedDrillTable({ runtime, track, active, tag, hour, tagAt, measure, refits, count, chartType, chartTitle, chartKey, onChart }: {
  runtime: WidgetRuntime; track: BoundTrack; active: boolean; tag: string; hour: number; tagAt: (hour: number) => string | null;
  measure: Measure; refits: boolean; count: Count; chartType: string | null; onChart: (type: string) => void;
  /** The chart below: what it stacks, and its parts with their colours and exact values. */
  chartTitle: string; chartKey: readonly { id: string; label: string; color?: string; value: number | null }[];
}) {
  const request = (counted: Count) => ({ sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced',
    params: requestParams(producedRequest(track.branch.id, { tags: [tag] }, measure, 'variant', [], refits, counted)) });
  const units = useProduct<EquipmentProducedReply>(runtime.client, request('units'), active, validateProducedReply);
  const ic = useProduct<EquipmentProducedReply>(runtime.client, request('ic'), active, validateProducedReply);
  const unitsModel = useMemo(() => units.data ? producedModel(units.data) : null, [units.data]);
  const icModel = useMemo(() => ic.data ? producedModel(ic.data) : null, [ic.data]);
  const icDecimals = countDecimals('ic', ic.data);
  const alias = tagAt(hour);
  const icTotal = icModel && alias ? producedAt(icModel, alias, hour)?.total ?? null : null;
  const rows = useMemo(() => alias ? typeRows(unitsModel, icModel, alias, hour, icTotal) : null, [unitsModel, icModel, alias, hour, icTotal]);
  const read = unitsModel ? readAt(unitsModel.reply.reads, hour) : null;
  const artwork = useStockpileArtwork(runtime, track, alias ?? tag, alias ? read : null, [], active);
  const pictures = useMemo(() => new Map(artwork?.data?.entries.map(entry => [entry.key, entry]) ?? []), [artwork]);
  const since = unitsModel ? countedSince(unitsModel.reply, hour) : null;
  const columns = useMemo<TableColumn<TypeRow>[]>(() => [
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.label,
      render: row => {
        const picture = pictures.get(`type:${row.type}`);
        return <span className={`produced-equipment${row.type === chartType ? ' is-charted' : ''}`} title={picture?.name ?? row.label}>
          {picture?.icon ? <img className="produced-art" src={picture.icon} alt="" /> : <ArtFallback icon="hammer" className="produced-art" />}
          <span><b>{row.label}</b>{row.models && <small>{row.models}</small>}</span>
        </span>;
      } },
    { key: 'units', header: 'Units', label: 'Units delivered', numeric: true, width: 'max-content', value: row => row.units,
      render: row => !row.recorded.units ? 'Not recorded' : row.units == null ? 'Not known' : formatRaw(row.units, 0), renderMissing: true },
    ...(icModel ? [{ key: 'ic', header: 'IC', label: 'IC put in', numeric: true, width: 'max-content', bar: 'visible' as const, value: (row: TypeRow) => row.ic,
      render: (row: TypeRow) => !row.recorded.ic ? 'Not recorded' : row.ic == null ? 'Not known' : <>{formatRaw(row.ic, icDecimals)} <small className="produced-share">{row.share}</small></>,
      renderMissing: true }] : []),
  ], [pictures, chartType, icModel, icDecimals]);
  const chartDecimals = countDecimals(count, count === 'ic' ? ic.data : units.data);
  const key = <>
    <p className="drill-empty produced-chart-title">{chartTitle}</p>
    <Legend always label={chartTitle} items={chartKey.map(part => ({ id: part.id, label: part.label, color: part.color,
      value: part.value == null ? 'Not known' : formatRaw(part.value, chartDecimals) }))} />
  </>;
  const window = producedWindow(measure, dayLabel(hour), since == null ? null : dayLabel(since));
  // The chart's key stays while the table's own replies load or refresh.
  const table = units.error ? <p className="drill-empty">{units.error}</p>
    : !unitsModel ? <p className="drill-empty">Loading what this country made…</p>
      : !alias ? <p className="drill-empty">Country unavailable at this time.</p>
        : <>
          <p className="drill-empty">Units delivered{icModel ? ' and IC put in' : ''} {window}, by type
            {icModel && icTotal != null && <> · All equipment <b>{formatRaw(icTotal, icDecimals)}</b> IC</>}</p>
          {!rows ? <p className="drill-empty">{producedWindowUnavailable(measure)}</p>
            : rows.length === 0 ? <p className="drill-empty">Nothing delivered {window}.</p>
            : <div className="produced-table" style={{ height: Math.min(rows.length * ROW_HEIGHT + 28, 320) }}>
              <Table label="Equipment produced by type" rows={rows} columns={columns} rowKey={row => row.type} rowHeight={ROW_HEIGHT}
                sort={{ column: count === 'ic' && icModel ? 'ic' : 'units', direction: 'descending' }} onRowClick={row => onChart(row.type)} />
            </div>}
        </>;
  return <div className="produced-drill">{table}{key}</div>;
}
