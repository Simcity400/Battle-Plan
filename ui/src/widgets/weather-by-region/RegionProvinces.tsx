import { useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { temperatureText, validateProvinces } from './weatherData';
import type { ProvinceWeather, ProvincesReply } from './weatherData';
import { type RegionAction } from '../../dashboard/regionTime';

const fixedText = (raw: string | null) => raw === null ? '—' : (Number(raw) / 100000).toLocaleString('en-US', { maximumFractionDigits: 2 });
const rowKey = (row: ProvinceWeather) => row.id;
const structure = (reply: ProvincesReply) => JSON.stringify(reply);

export default function RegionProvinces({ runtime, widget, region, time, active, onWarnings, onHour }: {
  runtime: WidgetRuntime; widget: WidgetSpec; region: string; time: RegionAction; active: boolean; onWarnings: (warnings: readonly string[]) => void;
  onHour?: (hour: number | null) => void;
}) {
  const params = useMemo(() => ({ region }), [region]);
  const read = useHeadProduct(runtime, useGate(runtime, widget, time), 'regional-weather-provinces', params, { active, validate: validateProvinces, structure });
  useEffect(() => { if (read.data) onWarnings(read.data.warnings); }, [read.data]);
  useEffect(() => { onHour?.(read.data?.readHour ?? read.hour); }, [read.data, read.hour]);
  useEffect(() => () => onHour?.(null), []);
  const columns = useMemo<readonly TableColumn<ProvinceWeather>[]>(() => [
    { key: 'name', header: 'Province', label: 'Province', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? row.id,
      render: row => row.name ?? `Province ${row.id}` },
    { key: 'temperature', header: 'Temperature', label: 'Temperature', numeric: true, width: 'max-content',
      value: row => row.temperature === null ? null : Number(row.temperature), render: row => row.temperature === null ? '—' : temperatureText(row.temperature) },
    { key: 'snow', header: 'Snow', label: 'Snow', numeric: true, width: 'max-content', priority: 2, value: row => row.snow === null ? null : Number(row.snow), render: row => fixedText(row.snow) },
    { key: 'mud', header: 'Mud', label: 'Mud', width: 'max-content', priority: 1, value: row => row.mud === null ? null : row.mud ? 1 : 0,
      render: row => row.mud === null ? '—' : row.mud ? 'Mud' : '' },
  ], []);
  if (read.error) return <div ref={read.root}><WidgetState kind="error" what="the region’s provinces" failure={read.error} onRetry={read.retry} /></div>;
  if (!read.data) return <div ref={read.root}><WidgetState kind="loading" label="Loading provinces…" read={read} /></div>;
  if (!read.data.provinces.length) return <p ref={read.root} className="drill-empty">No provinces are recorded for this region at this date.</p>;
  return <div ref={read.root}><Table label="Provinces of this region" rows={read.data.provinces} columns={columns} rowKey={rowKey}
    sort={{ column: 'temperature', direction: 'ascending' }} /></div>;
}
