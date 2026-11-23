import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useMemo, useRef, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import { dayLabel } from '../../charts/draw';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import WidgetState from '../../components/WidgetState';
import { Select } from '../../components/Select';
import type { WidgetProps } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { intelTooltip, intelValues, validateIntelDetails } from './intelData';
import type { IntelDetail, IntelDetailsReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

const keyOf = (row: IntelDetail) => row.key;
const SORT = { column: 'group', direction: 'ascending' } as const;

export default function IntelSection({ widget, runtime, active, hour, playing }: Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & { hour: number | null; playing: boolean }) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const selectionSource = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [choice, setChoice] = useState<{ source: string; country: number }>();
  const [viewerChoice, setViewerChoice] = useState<{ source: string; country: number }>();
  const country = choice?.source === selectionSource ? choice.country : null;
  const viewer = viewerChoice?.source === selectionSource ? viewerChoice.country : null;
  const [group, setGroup] = useState('All intel');
  const asked = useRef<number | null>(null);
  if (hour !== null && (!playing || asked.current === null)) asked.current = hour;
  const gate = useGate(runtime, widget, SPEC, active);
  const at = gate.recorded(asked.current);
  const validate = useMemo(() => (value: unknown) => {
    const data = validateIntelDetails(value);
    if (data.hour !== at || (country !== null && data.country !== country) || (viewer !== null && data.viewer !== viewer)) throw new Error('The intel reply belongs to another selection.');
    return data;
  }, [at, country, viewer]);
  const read = useProduct<IntelDetailsReply>(runtime.client, track && at !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'intel-details',
    params: { branchId: track.branch.id, hour: at, country, viewer } }) : null, active, validate, track?.source);
  const held = useRef<IntelDetailsReply | undefined>(undefined);
  if (read.data) held.current = read.data;
  const data = read.data ?? held.current;
  const labels = useMemo(() => new Map(data?.countries.map(item => [item.id, item.tag])), [data]);
  const countryOptions = useMemo(() => data?.countries.map(item => ({ value: String(item.id), label: item.tag })) ?? [], [data]);
  const rows = useMemo(() => data?.entries.filter(entry => group === 'All intel' || entry.group === group) ?? [], [data, group]);
  const groups = useMemo(() => ['All intel', ...new Set([...(data?.entries.map(entry => entry.group) ?? []), ...(group === 'All intel' ? [] : [group])])], [data, group]);
  const columns = useMemo<readonly TableColumn<IntelDetail>[]>(() => [
    { key: 'group', header: 'Intel', label: 'Intel', width: 'max-content', priority: 2, value: row => row.group },
    { key: 'pair', header: 'Viewer / target', label: 'Viewer / target', width: 'max-content', priority: 1,
      value: row => `${labels.get(row.viewer) ?? row.viewer} / ${row.target === null ? '—' : labels.get(row.target) ?? row.target}` },
    { key: 'entry', header: 'Entry', label: 'Entry', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.label },
    { key: 'value', header: 'Recorded values', label: 'Recorded values', width: 'minmax(var(--table-name-min), 3fr)', sortable: false,
      value: row => intelValues(row).map(value => `${value.name}: ${value.value}`).join(' · ') || 'Observed empty extent' },
  ], [labels]);
  if (!track) return null;
  return <section className="agency-intel-section" aria-label="Country intel">
    <div className="agency-intel-controls"><h3>Recorded intel records{data && hour !== null && data.hour !== hour && ` · ${dayLabel(data.hour)}`}</h3>
      {data && <><label>Viewing country <Select label="Intel viewing country" value={String(data.viewer ?? '')} options={countryOptions}
        onChange={value => setViewerChoice({ source: selectionSource, country: Number(value) })} /></label>
      <label>Country <Select label="Intel country" value={String(data.country ?? '')} options={countryOptions}
        onChange={value => setChoice({ source: selectionSource, country: Number(value) })} /></label>
      <label>Show <Select label="Intel entries" value={group} options={groups.map(item => ({ value: item, label: item }))} onChange={setGroup} /></label></>}
    </div>
    {read.error ? <WidgetState kind="error" failure={read.error} onRetry={read.retry} /> : !data ? <WidgetState kind="loading" label="Loading country intel…" /> : <>
      {data.warnings.length > 0 && <details className="agency-unknown"><summary>Unavailable intel inputs ({data.warnings.length})</summary>
        {data.warnings.map((warning, index) => <p key={index}>{warning}</p>)}</details>}
      <Table rowRevision={row => [row, columns]} label="Country intel" rows={rows} rowKey={keyOf} columns={columns} sort={SORT} density="compact"
        narrow={narrowTableOf(widget.options)} tooltip={intelTooltip} />
    </>}
  </section>;
}
