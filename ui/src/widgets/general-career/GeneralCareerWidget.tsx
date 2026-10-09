import WidgetState from '../../components/WidgetState';
import { useTableHour } from '../../charts/table/useTableHour';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';
import CareerView from './CareerView';

export default function GeneralCareerWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const options = resolveOptions(OPTIONS, widget.options);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a leader career." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign." />;
  if (!options.leader) return <WidgetState kind="empty" message="Choose a leader in this widget’s settings." />;
  return <CareerView {...props} reference={String(options.leader)} hour={Math.floor(window.lastHour)} />;
}
