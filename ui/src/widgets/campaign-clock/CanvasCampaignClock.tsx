import WidgetState from '../../components/WidgetState';
import CanvasTable from '../../charts/canvasTable/CanvasTable';
import type { CanvasColumn } from '../../charts/canvasTable/CanvasTable';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetProps } from '../../dashboard/definitions';
import { loadProduct } from '../../dashboard/products';
import { clockRequest } from '../../dashboard/widgetRequests';
import type { CampaignClockReply } from '../../generated/dashboard';
import { useLiveMoments } from '../../live/useLiveMoments';
import { CLOCK_NOTE, clockText, validateClock } from './clockData';

const COLUMNS: readonly CanvasColumn[] = [
  { key: 'clock', label: 'Clock', kind: 'text' },
  { key: 'time', label: 'Real time (h:mm:ss)', kind: 'numeric' },
];
interface ClockRow { name: string; time: string; note: string }

/** The campaign clock's four rows, painted in a canvas from hours read ahead of the playhead. */
export default function CanvasCampaignClock({ widget, runtime, active }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const content = useContentRevision(runtime.client);
  const lane = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const { store, phase } = useLiveMoments<ClockRow>({ active, lane, runtime, widget, track, content, countries: false,
    read: async (hour, signal, restart) => {
      // `hour` is the followed window's last hour; a followed clock counts from the branch's start.
      const request = clockRequest(widget, track, { firstHour: hour, lastHour: hour });
      if (!request) return { through: hour, rows: [], unavailable: 'This time is outside the recording.' };
      const data = await loadProduct<CampaignClockReply>(runtime.client, request, signal, validateClock, track!.source, restart, undefined, undefined, true);
      if (!data.available) return { through: hour, rows: [], unavailable: 'Playing and paused time were not recorded in this campaign.' };
      if (data.recordedSeconds === null) return { through: hour, rows: [], unavailable: 'Overlapping recordings prevent a unique campaign clock total.' };
      const note = data.warnings.length ? `${CLOCK_NOTE} ${data.warnings.join(' ')}` : CLOCK_NOTE;
      const row = (key: string, name: string, time: string) => ({ key, hour, cells: [{ text: name, bar: null }, { text: time, bar: null }], data: { name, time, note } });
      return { through: hour, rows: [
        row('total', 'Total recorded time', clockText(data.recordedSeconds)),
        row('playing', 'Playing time', clockText(data.playingSeconds)),
        row('paused', 'Paused time', clockText(data.pausedSeconds)),
        row('coverage', 'Coverage', data.complete ? 'Complete' : 'Gaps excluded'),
      ] };
    } });

  if (!track) return <WidgetState kind="empty" icon="clock" message="Load a campaign to see its recorded clock times." />;
  return <>
    {phase.state === 'error' && <WidgetState kind="error" what="campaign clock" failure={phase.failure} onRetry={store?.retry} />}
    {(phase.state === 'loading' || !store) && <WidgetState kind="loading" label="Loading campaign clock…" />}
    {phase.state === 'unavailable' && <WidgetState kind="unavailable" message={phase.message} />}
    {store && <div hidden={phase.state !== 'ready'} style={{ display: phase.state === 'ready' ? 'contents' : undefined }}>
      <CanvasTable label="Recorded campaign clock times" columns={COLUMNS} rank={false} density="comfortable" clock={runtime.clock} active={active}
        moment={store.moment} bind={store.bind}
        tooltip={row => ({ title: row.name, rows: [{ label: 'Real time', value: row.time }], note: row.note })} />
    </div>}
  </>;
}
