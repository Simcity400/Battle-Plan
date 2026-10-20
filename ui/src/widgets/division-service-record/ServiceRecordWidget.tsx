import WidgetState from '../../components/WidgetState';
import { logSettings } from '../../charts/log/logOptions';
import { useTableHour } from '../../charts/table/useTableHour';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';
import RecordView from './RecordView';

export default function ServiceRecordWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const options = resolveOptions(OPTIONS, widget.options);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a division’s record." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign." />;
  if (!options.division) return <WidgetState kind="empty" message="Choose a division." />;
  return <RecordView runtime={runtime} trackId={track.id} division={String(options.division)} hour={Math.floor(window.lastHour)}
    show={String(options.show)} log={logSettings(options)} live={widget.time.mode === 'follow'} active={active} onDiagnostics={onDiagnostics} />;
}
