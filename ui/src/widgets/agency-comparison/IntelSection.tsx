import { useMemo, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import { dayLabel } from '../../charts/draw';
import type { WidgetProps } from '../../dashboard/definitions';
import { useTableHour } from '../../charts/table/useTableHour';
import { resolveWindow } from '../../dashboard/model';
import { loadProduct } from '../../dashboard/products';
import { useViewRead } from '../../dashboard/viewRead';
import { useContentRevision } from '../../dashboard/contentRevision';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import WidgetState from '../../components/WidgetState';
import { Select } from '../../components/Select';
import type { IntelDetail, IntelDetailsReply } from '../../generated/dashboard';
import { intelTooltip, intelValues, validateIntelDetails } from './intelData';

const keyOf = (row: IntelDetail) => row.key;
/** A section of Agency comparison; its country choice includes countries without an agency. */
export default function IntelSection({ widget, runtime, active }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = window?.lastHour ?? track?.branch.lastHour ?? 0;
  const [choice, setChoice] = useState<{ source: string; country: number }>();
  const selectionSource = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const country = choice?.source === selectionSource ? choice?.country ?? null : null;
  const [viewerChoice, setViewerChoice] = useState<{ source: string; country: number }>();
  const viewer = viewerChoice?.source === selectionSource ? viewerChoice?.country ?? null : null;
  const [group, setGroup] = useState('All intel');
  const revision = useContentRevision(runtime.client);
  // Hour follows playback (`useViewRead`): the last landed hour stays shown while the newest is read.
  const lane = track && window ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, country, viewer, revision]) : null;
  const read = useViewRead<number, IntelDetailsReply>(lane, hour, String(hour), active, (asked, signal, publish, restart) => {
    const request = { sourceId: track!.source.id, revision: track!.source.revision, product: 'intel-details', params: { branchId: track!.branch.id, hour: asked, country, viewer } };
    const validate = (reply: unknown) => {
      const data = validateIntelDetails(reply);
      if (data.hour !== asked || (country !== null && data.country !== country) || (viewer !== null && data.viewer !== viewer)) throw new Error('The intel reply belongs to another selection.');
      return data;
    };
    return loadProduct(runtime.client, request, signal, validate, undefined, restart, publish, undefined, true);
  }, runtime.client);
  const data = read.data;
  const labels = useMemo(() => new Map(data?.countries.map(country => [country.id, country.tag])), [data]);
  const countryOptions = useMemo(() => data?.countries.map(country => ({ value: String(country.id), label: country.tag })) ?? [], [data]);
  const rows = useMemo(() => data?.entries.filter(entry => group === 'All intel' || entry.group === group) ?? [], [data, group]);
  const groups = useMemo(() => ['All intel', ...new Set([...(data?.entries.map(entry => entry.group) ?? []), ...(group === 'All intel' ? [] : [group])])], [data, group]);
  const columns = useMemo<readonly TableColumn<IntelDetail>[]>(() => [
    { key: 'group', header: 'Intel', label: 'Intel', width: 'max-content', priority: 2, value: row => row.group },
    { key: 'pair', header: 'Viewer / target', label: 'Viewer / target', width: 'max-content', priority: 1, value: row => `${labels.get(row.viewer) ?? row.viewer} / ${row.target === null ? '—' : labels.get(row.target) ?? row.target}` },
    { key: 'entry', header: 'Entry', label: 'Entry', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.label },
    { key: 'value', header: 'Recorded values', label: 'Recorded values', width: 'minmax(var(--table-name-min), 3fr)', value: row => intelValues(row).map(value => `${value.name}: ${value.value}`).join(' · ') || 'Observed empty extent', sortable: false },
  ], [labels]);
  if (!track || !window) return null;
  return <section className="agency-intel-section" aria-label="Country intel">
    <div className="agency-intel-controls"><h3>Recorded intel records{data && data.hour !== Math.floor(hour) && ` · ${dayLabel(data.hour)}`}</h3>
      {data && <><label>Viewing country <Select label="Intel viewing country" value={String(data.viewer ?? '')} options={countryOptions}
        onChange={value => setViewerChoice({ source: selectionSource, country: Number(value) })} /></label>
      <label>Country <Select label="Intel country" value={String(data.country ?? '')} options={countryOptions}
        onChange={value => setChoice({ source: selectionSource, country: Number(value) })} /></label>
      <label>Show <Select label="Intel entries" value={group} options={groups.map(group => ({ value: group, label: group }))} onChange={setGroup} /></label></>}
    </div>
    {read.error ? <WidgetState kind="error" failure={read.error} onRetry={read.retry} /> : !data ? <WidgetState kind="loading" label="Loading country intel…" /> : <>
      {data.warnings.length > 0 && <details className="agency-unknown"><summary>Unavailable intel inputs ({data.warnings.length})</summary>{data.warnings.map((warning, index) => <p key={index}>{warning}</p>)}</details>}
      <Table rowRevision={row => [row, columns]} label="Country intel" rows={rows} rowKey={keyOf} columns={columns} sort={{ column: 'group', direction: 'ascending' }} density="compact" narrow={narrowTableOf(widget.options)} tooltip={intelTooltip} parity={{ complete: true, item: row => row }} />
    </>}
  </section>;
}
