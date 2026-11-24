import type { Gate } from '../../dashboard/regionGate';
import { answersKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import { failureText } from '../../dashboard/queryError.ts';
import type { NarrowTable } from '../../charts/table/options';
import ArtFallback from '../../components/ArtFallback';
import Legend from '../../charts/Legend';
import { dayLabel } from '../../charts/draw';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { formatRaw } from './producedData';
import type { Count, Measure } from './producedData';
import { amountText, producedWindow, producedWindowUnavailable, typeRowRevision, typeRows } from './producedView';
import type { TypeRow } from './producedView';
import { useProducedArtwork } from './producedArtwork';
import { useProducedAt } from './useProduced';

const ROW_HEIGHT = 40;
const IC_DECIMALS = 5;

export default function ProducedDrillTable({ runtime, gate, track, active, tag, hour, tagAt, measure, refits, count, chartType, chartTitle, chartKey, onChart, narrow }: {
  runtime: WidgetRuntime; gate: Pick<Gate, 'recorded'>; track: BoundTrack; active: boolean; tag: string; hour: number; tagAt: (hour: number) => string | null; narrow: NarrowTable;
  measure: Measure; refits: boolean; count: Count; chartType: string | null; onChart: (type: string) => void;
  chartTitle: string; chartKey: readonly { id: string; label: string; color?: string; value: number | null }[];
}) {
  const alias = tagAt(hour);
  const at = useProducedAt(runtime, track, alias, alias ? gate.recorded(hour) : null, measure, refits, active);
  const reply = at.data && answersKey(at.data.asked, alias) && at.data.hour === Math.floor(hour) ? at.data : undefined;
  const rows = useMemo(() => reply ? typeRows(reply) : null, [reply]);
  const pictures = useProducedArtwork(runtime, track, alias ?? tag, gate.recorded(reply?.read), active);
  const since = reply?.observation ?? null;
  const columns = useMemo<TableColumn<TypeRow>[]>(() => [
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.label,
      render: row => {
        const picture = pictures.get(`type:${row.type}`);
        return <span className={`produced-equipment${row.type === chartType ? ' is-charted' : ''}`} title={picture?.name ?? row.label}>
          {picture?.icon ? <img className="produced-art" src={picture.icon} alt="" /> : <ArtFallback icon="hammer" name={picture?.name ?? row.label} className="produced-art" />}
          <span><b>{row.label}</b>{row.models && <small>{row.models}</small>}</span>
        </span>;
      } },
    { key: 'units', header: 'Units', label: 'Units delivered', numeric: true, width: 'max-content', priority: 1, widthKey: row => [row.units, row.recorded], value: row => row.units,
      text: row => !row.recorded ? 'Not recorded' : row.units == null ? 'Not known' : formatRaw(row.units, 0),
      render: row => !row.recorded ? 'Not recorded' : row.units == null ? <span title={row.why}>Not known</span> : formatRaw(row.units, 0), renderMissing: true },
    { key: 'ic', header: 'IC', label: 'IC put in', numeric: true, width: 'max-content', bar: 'visible' as const, widthKey: (row: TypeRow) => [row.ic, row.share, row.recorded], value: (row: TypeRow) => row.ic,
      render: (row: TypeRow) => !row.recorded ? 'Not recorded' : row.ic == null ? <span title={row.why}>Not known</span> : <>{formatRaw(row.ic, IC_DECIMALS)} <small className="produced-share">{row.share}</small></>,
      renderMissing: true },
  ], [pictures, chartType]);
  const chartDecimals = count === 'ic' ? IC_DECIMALS : 0;
  const key = <>
    <p className="drill-empty produced-chart-title">{chartTitle}</p>
    <Legend always label={chartTitle} items={chartKey.map(part => ({ id: part.id, label: part.label, color: part.color,
      value: part.value == null ? 'Not known' : formatRaw(part.value, chartDecimals) }))} />
  </>;
  const window = producedWindow(measure, dayLabel(hour), since == null ? null : dayLabel(since));
  const table = !alias ? <p className="drill-empty">Country unavailable at this time.</p>
    : at.error ? <p className="drill-empty" data-detail={at.error.message}>{failureText(at.error, 'Couldn’t load what this country made.')}</p>
      : !reply ? <p className="drill-empty">Loading what this country made…</p>
        : <>
          <p className="drill-empty">Units delivered and IC put in {window}, by type
            {reply.recorded && <> · All equipment <b>{amountText(reply.icTotal, IC_DECIMALS)}</b> IC</>}</p>
          {!rows ? <p className="drill-empty">{producedWindowUnavailable(measure)}</p>
            : rows.length === 0 ? <p className="drill-empty">Nothing delivered {window}.</p>
              : <div className="produced-table" style={{ height: Math.min(rows.length * ROW_HEIGHT + 28, 320) }}>
                <Table rowRevision={row => [...typeRowRevision(row), row.type === chartType, pictures.get(`type:${row.type}`)?.icon]} label="Equipment produced by type"
                  rows={rows} columns={columns} rowKey={row => row.type} rowHeight={ROW_HEIGHT} narrow={narrow}
                  sort={{ column: count === 'ic' ? 'ic' : 'units', direction: 'descending' }} onRowClick={row => onChart(row.type)} />
              </div>}
        </>;
  return <div className="produced-drill">{table}{key}</div>;
}
