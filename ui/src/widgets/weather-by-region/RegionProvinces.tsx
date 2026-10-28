import { useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { loadTransientProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import * as requests from '../../dashboard/batch5Requests';
import { temperatureText, validateProvinces } from './weatherData';
import type { ProvinceWeather, ProvincesReply } from './weatherData';

/** Snow as the recording stores it, on the game's fixed-point scale. */
const fixedText = (raw: string | null) => raw === null ? '—' : (Number(raw) / 100000).toLocaleString('en-US', { maximumFractionDigits: 2 });
const rowKey = (row: ProvinceWeather) => row.id;

/** One region's provinces at the drill's hour, read on demand. */
export default function RegionProvinces({ runtime, track, region, hour, active, onWarnings }: {
  runtime: WidgetRuntime; track: BoundTrack; region: string; hour: number; active: boolean; onWarnings: (warnings: readonly string[]) => void;
}) {
  const content = useContentRevision(runtime.client);
  const [state, setState] = useState<{ client: WidgetRuntime['client']; key: string; data?: ProvincesReply; error?: string }>();
  const request = useMemo(() => ({ sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather',
    params: requests.weatherProvinces(track.branch.id, region, hour) }), [track, region, hour]);
  const key = JSON.stringify([request, content]);
  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal, track.source).then(reply => {
      if (controller.signal.aborted) return;
      const data = validateProvinces(reply);
      setState({ client: runtime.client, key, data });
      onWarnings(data.warnings);
    }).catch(error => { if (!controller.signal.aborted) setState({ client: runtime.client, key, error: error instanceof Error ? error.message : String(error) }); });
    return () => controller.abort();
    // The request's identity is its key, with the content binding it was read under.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key, active]);
  const columns = useMemo<readonly TableColumn<ProvinceWeather>[]>(() => [
    { key: 'name', header: 'Province', label: 'Province', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? row.id,
      render: row => row.name ?? 'Province name unavailable' },
    { key: 'temperature', header: 'Temperature', label: 'Temperature', numeric: true, width: 'max-content',
      value: row => row.temperature === null ? null : Number(row.temperature), render: row => row.temperature === null ? '—' : temperatureText(row.temperature) },
    { key: 'snow', header: 'Snow', label: 'Snow', numeric: true, width: 'max-content', priority: 2, value: row => row.snow === null ? null : Number(row.snow), render: row => fixedText(row.snow) },
    { key: 'mud', header: 'Mud', label: 'Mud', width: 'max-content', priority: 1, value: row => row.mud === null ? null : row.mud ? 1 : 0,
      render: row => row.mud === null ? '—' : row.mud ? 'Mud' : '' },
  ], []);
  const current = state?.client === runtime.client && state.key === key ? state : undefined;
  if (current?.error) return <WidgetState kind="error" what="the region's provinces" detail={current.error} />;
  if (!current?.data) return <WidgetState kind="loading" label="Loading provinces…" />;
  if (!current.data.provinces.length) return <p className="drill-empty">No provinces are recorded for this region at this date.</p>;
  return <Table label="Provinces of this region" rows={current.data.provinces} columns={columns} rowKey={rowKey}
    sort={{ column: 'temperature', direction: 'ascending' }} />;
}
