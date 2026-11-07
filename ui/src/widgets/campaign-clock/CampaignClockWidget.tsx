import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { useTableHour } from '../../charts/table/useTableHour';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useFollowedProduct } from '../../dashboard/products';
import type { CampaignClockReply } from '../../generated/dashboard';
import { clockRequest } from '../../dashboard/widgetRequests';
import { CLOCK_NOTE, clockText, validateClock } from './clockData';

interface ClockRow { id: string; name: string; seconds: number | null; text?: string; note: string }
const NO_WARNINGS: string[] = [];
const rowKey = (row: ClockRow) => row.id;
const SORT = { column: 'clock', direction: 'ascending' } as const;
const COLUMNS: readonly TableColumn<ClockRow>[] = [
  { key: 'clock', header: 'Clock', label: 'Clock', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.name },
  { key: 'time', header: 'Real time (h:mm:ss)', label: 'Real time', width: 'max-content', align: 'end', numeric: true,
    value: row => row.seconds, renderMissing: true, render: row => row.text ?? clockText(row.seconds) },
];

export default function CampaignClockWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const hour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, hour);
  const request = clockRequest(widget, track, window ?? null);
  // While the newest hours are read, every row, coverage too, is the shown reply's own.
  const { data, error, retry } = useFollowedProduct<CampaignClockReply>(runtime.client, request,
    ['firstHour', 'lastHour'], active, validateClock, track?.source);
  const note = data?.warnings.length ? `${CLOCK_NOTE} ${data.warnings.join(' ')}` : CLOCK_NOTE;
  const rows = useMemo<ClockRow[]>(() => data ? [
    { id: 'total', name: 'Total recorded time', seconds: data.recordedSeconds, note },
    { id: 'playing', name: 'Playing time', seconds: data.playingSeconds, note },
    { id: 'paused', name: 'Paused time', seconds: data.pausedSeconds, note },
    { id: 'coverage', name: 'Coverage', seconds: null,
      text: data.complete ? 'Complete' : 'Gaps excluded', note },
  ] : [], [data, note]);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);

  if (!track) return <WidgetState kind="empty" icon="clock" message="Load a campaign to see its recorded clock times." />;
  if (!request) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" what="campaign clock" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading campaign clock…" />;
  if (!data.available) return <WidgetState kind="unavailable" message="Playing and paused time were not recorded in this campaign." />;
  if (data.recordedSeconds === null) return <WidgetState kind="unavailable" message="Overlapping recordings prevent a unique campaign clock total." />;
  return <Table rowRevision={row => [row, COLUMNS]} label="Recorded campaign clock times" rows={rows} columns={COLUMNS} rowKey={rowKey} unsorted
    sort={SORT} cellBars={false} tooltip={row => ({ title: row.name, rows: [{ label: 'Real time', value: row.text ?? clockText(row.seconds) }], note: row.note })} />;
}
