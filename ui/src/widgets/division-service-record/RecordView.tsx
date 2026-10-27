import { useMemo } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import type { LogSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import { useReachedProduct } from '../../dashboard/reachedProduct';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { divisionName, entryText, entryTooltip, headerDetail, recordRows, validateDivisionRecord } from './careersData';
import type { RecordItem } from './careersData';
import './serviceRecord.css';

/** One division's header line and dated record: the standalone widget's body and
 * the Most decorated divisions drill. */
export default function RecordView({ runtime, trackId, division, hour, show, log, live, active, onDiagnostics, caption }: {
  runtime: WidgetRuntime; trackId: string; division: string; hour: number; show: string; log: LogSettings; live: boolean; active: boolean;
  onDiagnostics: WidgetProps['onDiagnostics']; caption?: string;
}) {
  const track = runtime.tracks.find(t => t.id === trackId);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'division-record',
    params: { branchId: track.branch.id, hour, division } } : null;
  const state = useReachedProduct(runtime.client, track?.source, request, JSON.stringify([track?.branch.id, division]), active, validateDivisionRecord);
  const data = state.data;
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const rows = useMemo(() => data ? recordRows(data, show) : [], [data, show]);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a division’s record." />;
  if (state.error) return <WidgetState kind="error" detail={state.error} onRetry={state.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the record…" />;
  if (data.absent) return <WidgetState kind="unavailable" message="This division is not in service at this date." />;
  const name = divisionName(data.name);
  const renderRow = ({ item }: LogRow<RecordItem>) => <><span className="log-art" aria-hidden="true">
    {item.entry.award ? <ArtFallback icon="star" /> : data.template?.icon ? <img src={data.template.icon} alt="" /> : <ArtFallback icon="helmet" />}</span>
    <span className="log-text">{entryText(item)}</span></>;
  return <div className="service-record">
    {caption && <p className="service-record-subject">{caption}</p>}
    <div className="service-record-header" data-tooltip={data.division}>
      {data.template?.icon ? <img src={data.template.icon} alt="" /> : <ArtFallback icon="helmet" />}
      <strong title={name}>{name}</strong>
      <small title={headerDetail(data.template?.name ?? null, data.facts)}>{headerDetail(data.template?.name ?? null, data.facts)}</small>
    </div>
    <LogList rows={rows} label={`${name} service record`} density={log.density} months={log.months} animate={log.animate && live}
      playhead={live ? hour : null} columns="var(--space-6) minmax(0, 1fr)" renderRow={renderRow}
      describe={({ item }) => `${formatDateTime(item.entry.hour)}: ${entryText(item)}`}
      tooltip={({ item }) => [entryTooltip(item)]} menu={({ item }) => [jumpItem(runtime, track.id, item.entry.hour)]}
      empty={data.facts?.historyMissing ? 'This division’s history wasn’t recorded at this time.' : `${name} has no recorded career yet.`} />
  </div>;
}
