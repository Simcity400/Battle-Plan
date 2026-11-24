import type { ReactNode } from 'react';
import { useEffect, useMemo } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import type { LogSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { divisionName, entryText, entryTooltip, headerDetail, recordRows, recordStructure, validateDivisionRecord } from './careersData';
import type { RecordItem } from './careersData';
import './serviceRecord.css';
import { regionFollows, type RegionAction } from '../../dashboard/regionTime';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime';

export default function RecordView({ runtime, widget, trackId, division, time, show, log, active, onDiagnostics, caption, onAnswered }: {
  runtime: WidgetRuntime; widget: WidgetSpec; trackId: string; division: string; time: RegionAction; show: string; log: LogSettings; active: boolean;
  onDiagnostics: WidgetProps['onDiagnostics']; caption?: ReactNode;
  onAnswered?: (hour: number | undefined) => void;
}) {
  const track = runtime.tracks.find(t => t.id === trackId);
  const region = useRegion(widget, time);
  const live = regionFollows(region);
  const hour = useRegionHour(runtime, region, active);
  const state = useHeadProduct(runtime, useGate(runtime, widget, time), 'division-record', track ? { division } : null,
    { active, validate: validateDivisionRecord, structure: recordStructure });
  const data = state.data;
  const answered = data === undefined ? undefined : state.at ?? undefined;
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  useEffect(() => {
    onAnswered?.(answered);
    return () => onAnswered?.(undefined);
  }, [onAnswered, answered]);
  const rows = useMemo(() => data ? recordRows(data, show) : [], [data, show]);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a division’s record." />;
  if (state.outside) return <WidgetState kind="outside" read={state} />;
  if (state.error) return <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the record…" read={state} />;
  if (data.absent) return <WidgetState kind="unavailable" message="This division is not in service at this date." />;
  const name = divisionName(data.name);
  const renderRow = ({ item }: LogRow<RecordItem>) => <><span className="log-art" aria-hidden={!item.entry.award && data.template?.icon ? true : undefined}>
    {item.entry.award ? <ArtFallback icon="star" name={entryText(item)} /> : data.template?.icon ? <img src={data.template.icon} alt="" /> : <ArtFallback icon="helmet" name={data.template?.name ?? name} />}</span>
    <span className="log-text">{entryText(item)}</span></>;
  return <div className="service-record" ref={state.root}>
    {caption && <p className="service-record-subject">{caption}</p>}
    <div className="service-record-header" data-tooltip={data.division}>
      {data.template?.icon ? <img src={data.template.icon} alt="" /> : <ArtFallback icon="helmet" name={data.template?.name ?? name} />}
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
