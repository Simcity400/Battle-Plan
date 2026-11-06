import { useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useFollowedProduct } from '../../dashboard/products';
import * as requests from '../../dashboard/batch5Requests';
import { temperatureText, validateProvinces } from './weatherData';
import type { ProvinceWeather, ProvincesReply } from './weatherData';

/** Snow as the recording stores it, on the game's fixed-point scale. */
const fixedText = (raw: string | null) => raw === null ? '—' : (Number(raw) / 100000).toLocaleString('en-US', { maximumFractionDigits: 2 });
const rowKey = (row: ProvinceWeather) => row.id;

const MOVING_HOUR = ['hour'] as const;

/** One region's provinces at the drill's hour, read on demand. While the drill follows playback,
 * the last landed hour stays shown while the newest is read; `onHour` names the hour shown. */
export default function RegionProvinces({ runtime, track, region, hour, active, onWarnings, onHour }: {
  runtime: WidgetRuntime; track: BoundTrack; region: string; hour: number; active: boolean; onWarnings: (warnings: readonly string[]) => void;
  onHour?: (hour: number | null) => void;
}) {
  const request = useMemo(() => ({ sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather',
    params: requests.weatherProvinces(track.branch.id, region, hour) }), [track, region, hour]);
  const read = useFollowedProduct<ProvincesReply>(runtime.client, request, MOVING_HOUR, active, validateProvinces, track.source);
  const answered = read.data === undefined || read.moment === undefined ? null : (JSON.parse(read.moment) as [number])[0];
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { if (read.data) onWarnings(read.data.warnings); }, [read.data]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { onHour?.(answered); }, [answered]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => () => onHour?.(null), []);
  const columns = useMemo<readonly TableColumn<ProvinceWeather>[]>(() => [
    { key: 'name', header: 'Province', label: 'Province', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? row.id,
      render: row => row.name ?? 'Province name unavailable' },
    { key: 'temperature', header: 'Temperature', label: 'Temperature', numeric: true, width: 'max-content',
      value: row => row.temperature === null ? null : Number(row.temperature), render: row => row.temperature === null ? '—' : temperatureText(row.temperature) },
    { key: 'snow', header: 'Snow', label: 'Snow', numeric: true, width: 'max-content', priority: 2, value: row => row.snow === null ? null : Number(row.snow), render: row => fixedText(row.snow) },
    { key: 'mud', header: 'Mud', label: 'Mud', width: 'max-content', priority: 1, value: row => row.mud === null ? null : row.mud ? 1 : 0,
      render: row => row.mud === null ? '—' : row.mud ? 'Mud' : '' },
  ], []);
  if (read.error) return <WidgetState kind="error" what="the region’s provinces" failure={read.error} />;
  if (!read.data) return <WidgetState kind="loading" label="Loading provinces…" />;
  if (!read.data.provinces.length) return <p className="drill-empty">No provinces are recorded for this region at this date.</p>;
  return <Table rowRevision={row => [row, columns]} label="Provinces of this region" rows={read.data.provinces} columns={columns} rowKey={rowKey}
    sort={{ column: 'temperature', direction: 'ascending' }} />;
}
