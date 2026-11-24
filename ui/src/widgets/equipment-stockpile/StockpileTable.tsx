import { useMemo, useRef } from 'react';
import type { NarrowTable } from '../../charts/table/options';
import ArtFallback from '../../components/ArtFallback';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { liveKey, mapLive, valuesOf } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { SortValue } from '../../charts/table/logic';
import { formatAmount, formatRate, formatSigned } from '../../charts/valueAxis';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { WidgetProps } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { AI_NOTE, balanceMode, formatPercent, groupTooltip, stockGroups } from './stockpileData';
import type { StockGroup } from './stockpileData';
import type { StockpileMode } from './stockpileDrill';
import { stockRows } from './stockpileView';
import type { StockCount, StockRow } from './stockpileView';
import { useStockpileArtwork } from './useStockpileArtwork';
import type { EquipmentStockpileReply } from './types';

const ROW_HEIGHT = 40;
const NOT_RECORDED = 'Not recorded';

export function paintedCells(rows: readonly StockRow[]): Map<string, SortValue> {
  const cells = new Map<string, SortValue>();
  for (const row of rows) {
    const key = row.group.key;
    if (row.surplus != null) cells.set(liveKey(key, 'surplus'), row.surplus);
    if (row.stock != null) cells.set(liveKey(key, 'stock'), row.stock);
    if (row.output != null) cells.set(liveKey(key, 'output'), row.output);
    cells.set(liveKey(key, 'models'), row.models);
  }
  return cells;
}

export default function StockpileTable({ data, hour, at, live, shownAt, included, count, narrow, active, runtime, track, widget, viewport, onDiagnostics, onPick }: {
  data: EquipmentStockpileReply; hour: number; at: (values: LiveValues<number>) => EquipmentStockpileReply | undefined;
  live: Live<LiveValues<number>>; shownAt: () => number | null;
  included: readonly string[]; count: StockCount; narrow: NarrowTable;
  active: boolean; runtime: WidgetProps['runtime']; track: BoundTrack; widget: WidgetSpec; viewport: number; onDiagnostics: WidgetProps['onDiagnostics'];
  onPick: (group: StockGroup, hour: number, mode: StockpileMode) => void;
}) {
  const mode = balanceMode(data, hour);
  const player = mode === 'player';
  const groups = useMemo(() => stockGroups(data, 'type', hour, included), [data, hour, included]);
  const { rows, quiet, unrecorded } = useMemo(() => stockRows(data, groups, hour, mode, count), [data, groups, hour, mode, count]);
  const artwork = useStockpileArtwork(runtime, widget, data.tag, [], active);
  const art = useMemo(() => new Map(artwork.data?.entries.map(entry => [entry.key, entry]) ?? []), [artwork.data]);
  const diagnostics = useMemo(() => [...data.warnings, ...(artwork.data?.warnings ?? [])], [data, artwork.data]);
  useDiagnostics(onDiagnostics, diagnostics);
  const shown = useShownHour();
  const shownNow = useRef(shownAt);
  shownNow.current = shownAt;
  const make = useRef(at);
  make.current = at;
  const derived = useRef<Map<string, StockRow>>(new Map());
  const painted = useMemo(() => mapLive(reporting(live, () => shown(shownNow.current())), values => {
    const view = make.current(values);
    if (!view) return valuesOf(new Map<string, SortValue>(), NOT_RECORDED);
    const made = stockRows(view, stockGroups(view, 'type', hour, included), hour, balanceMode(view, hour), count);
    derived.current = new Map(made.rows.map(row => [row.group.key, row]));
    return valuesOf(paintedCells(made.rows), (key: string) => key.endsWith('surplus') ? 'Not known' : NOT_RECORDED);
  }), [live, shown, included, count, hour]);
  const columns = useMemo<TableColumn<StockRow>[]>(() => [
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.group.label,
      render: row => {
        const picture = art.get(`type:${row.group.key}`);
        return <span className="stockpile-equipment">
          {picture?.icon ? <img className="stockpile-art" src={picture.icon} alt="" /> : <span className="stockpile-art"><ArtFallback icon="hammer" name={row.group.label} /></span>}
          <span><b>{row.group.label}</b></span>
        </span>;
      } },
    { key: 'models', header: 'Models', label: 'Models', width: 'minmax(80px, 1fr)', priority: 2, value: row => row.models,
      paint: value => String(value) },
    player
      ? { key: 'surplus', header: 'Surplus / shortage', label: 'Surplus or shortage', numeric: true, width: 'max-content', widthKey: row => [row.surplus, row.share, data.decimals], value: row => row.surplus,
        hint: 'The game’s Logistics figure: available minus needed', renderMissing: true,
        paint: (value, key) => {
          const row = derived.current.get(key.split('\u0000')[0]);
          const share = row?.share;
          return `${formatSigned(BigInt(value), data.decimals, 0)}${share != null ? ` · ${formatPercent(share)} of need` : ''}`;
        } }
      : { key: 'stock', header: 'In stock', label: 'In stock', numeric: true, width: 'max-content', widthKey: row => [row.stock, data.decimals], value: row => row.stock, renderMissing: true,
        paint: value => formatAmount(BigInt(value), data.decimals, 0) },
    { key: 'output', header: count === 'ic' ? 'IC / day' : 'Output / day', label: count === 'ic' ? 'IC a day' : 'Output a day', numeric: true, width: 'max-content', priority: 1,
      text: row => row.output == null ? NOT_RECORDED : formatRate(row.output, data.decimals),
      widthKey: row => [row.output, data.decimals], value: row => row.output, renderMissing: true,
      paint: value => formatRate(BigInt(value), data.decimals) },
  ], [art, player, count, data.decimals]);
  const date = dayLabel(hour);
  return <div className="stockpile-table">
    <div className="stockpile-caption-line">
      <span>On {date} · {player ? 'surplus or shortage from the game’s Logistics screen' : mode === 'ai' ? AI_NOTE : 'in stock'}</span>
    </div>
    {rows.length === 0 ? <p className="stockpile-empty">Nothing of the included types was recorded on {date}.</p>
      : <div className="stockpile-table-rows" style={{ height: Math.min(rows.length * ROW_HEIGHT + 28, Math.max(160, viewport)) }}>
        <Table rowRevision={row => [row.group.label, player, count, data.decimals, art.get(`type:${row.group.key}`)?.icon]} label={`Equipment stockpile on ${date}`}
          rows={rows} columns={columns} rowKey={row => row.group.key} rowHeight={ROW_HEIGHT} narrow={narrow} live={painted}
          sort={player ? { column: 'surplus', direction: 'ascending' } : { column: 'stock', direction: 'descending' }}
          onRowClick={row => onPick(row.group, hour, player ? 'balance' : 'stock')}
          tooltip={row => groupTooltip(data, row.group, 'type', hour, { title: row.group.label, subtitle: date,
            note: art.get(`type:${row.group.key}`)?.name ?? undefined }, viewport)}
          menu={() => [jumpItem(runtime, track.id, hour)]} />
      </div>}
    {quiet > 0 && <p className="stockpile-empty">{quiet.toLocaleString()} more {quiet === 1 ? 'type' : 'types'}: nothing in stock, being made or needed on {date}.</p>}
    {unrecorded > 0 && <p className="stockpile-empty">{unrecorded.toLocaleString()} more {unrecorded === 1 ? 'type' : 'types'}: stock, production or need not recorded on {date}.</p>}
  </div>;
}
