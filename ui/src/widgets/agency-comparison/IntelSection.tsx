import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { useTableHour } from '../../charts/table/useTableHour';
import { resolveWindow } from '../../dashboard/model';
import { loadTransientProduct } from '../../dashboard/products';
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
  const request = track && window ? { sourceId: track.source.id, revision: track.source.revision, product: 'intel-details', params: { branchId: track.branch.id, hour, country, viewer } } : null;
  const key = JSON.stringify([request, revision]);
  const [state, setState] = useState<{ client: WidgetProps['runtime']['client']; key: string; data?: IntelDetailsReply; error?: Failure }>();
  const [attempt, retry] = useState(0);
  useEffect(() => {
    if (!active || !request) return;
    const controller = new AbortController();
    void loadTransientProduct<IntelDetailsReply>(runtime.client, request, controller.signal).then(reply => {
      const data = validateIntelDetails(reply);
      if (data.hour !== request.params.hour || (country !== null && data.country !== country) || (viewer !== null && data.viewer !== viewer)) throw new Error('The intel reply belongs to another selection.');
      if (!controller.signal.aborted) setState({ client: runtime.client, key, data });
    }).catch(error => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, error: asFailure(error) });
    });
    return () => controller.abort();
    // Identity includes hour, country, source revision and recorded content binding.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key, active, attempt]);
  const current = state?.client === runtime.client && state.key === key ? state : undefined;
  const data = current?.data;
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
    <div className="agency-intel-controls"><h3>Recorded intel records</h3>
      {data && <><label>Viewing country <Select label="Intel viewing country" value={String(data.viewer ?? '')} options={countryOptions}
        onChange={value => setViewerChoice({ source: selectionSource, country: Number(value) })} /></label>
      <label>Country <Select label="Intel country" value={String(data.country ?? '')} options={countryOptions}
        onChange={value => setChoice({ source: selectionSource, country: Number(value) })} /></label>
      <label>Show <Select label="Intel entries" value={group} options={groups.map(group => ({ value: group, label: group }))} onChange={setGroup} /></label></>}
    </div>
    {current?.error ? <WidgetState kind="error" failure={current.error} onRetry={() => retry(value => value + 1)} /> : !data ? <WidgetState kind="loading" label="Loading country intel…" /> : <>
      {data.warnings.length > 0 && <details className="agency-unknown"><summary>Unavailable intel inputs ({data.warnings.length})</summary>{data.warnings.map((warning, index) => <p key={index}>{warning}</p>)}</details>}
      <Table label="Country intel" rows={rows} rowKey={keyOf} columns={columns} sort={{ column: 'group', direction: 'ascending' }} density="compact" narrow={narrowTableOf(widget.options)} tooltip={intelTooltip} parity={{ complete: true, item: row => row }} />
    </>}
  </section>;
}

