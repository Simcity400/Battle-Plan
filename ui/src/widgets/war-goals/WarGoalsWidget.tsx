import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { selectedTags } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import WarGoalsPanel from './WarGoalsPanel';
export default function WarGoalsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see war goals." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose a recorded time." />;
  return <div className="timeline-scroll"><WarGoalsPanel runtime={runtime} track={track} tag={selectedTags(widget.countries, track)[0] ?? null}
    hour={Math.floor(window.lastHour)} active={active} expanded onWarnings={onDiagnostics} /></div>;
}
