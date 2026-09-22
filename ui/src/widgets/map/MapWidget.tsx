import { useCallback, useLayoutEffect, useMemo, useRef } from 'react';
import { Icon } from '../../components/Icons';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { useMapState } from '../../dashboard/geometry';
import type { MapTarget } from '../../dashboard/geometry';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import type { PlaybackController } from '../../dashboard/playback';

/** The native map's body: the renderer draws into this rectangle; React shows its state. */
export default function MapWidget({ widget, runtime, active, mapOptions }: WidgetProps) {
  const body = useRef<HTMLDivElement>(null);
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const staticSequence = useMemo(() => runtime.clock.getSnapshot().sequence, [runtime.clock, widget, track]);
  const target = useCallback((clock: ReturnType<PlaybackController['getSnapshot']>): MapTarget | null => {
    if (!track) return null;
    const window = resolveWindow(widget, track, runtime.tracks, clock.position);
    if (!window || window.lastHour < track.branch.firstHour || window.lastHour > track.branch.lastHour) return null;
    return { options: { fog: mapOptions.fog, country: widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : track.selectedTag }, sourceId: track.source.id, path: track.source.path, revision: track.source.revision, branchId: track.branch.id,
      hour: window.lastHour, fraction: widget.time.mode === 'follow' ? clock.fraction : 0,
      sequence: widget.time.mode === 'follow' ? clock.sequence : staticSequence };
  }, [track, widget, runtime.tracks, staticSequence, mapOptions]);
  useLayoutEffect(() => body.current ? runtime.maps.register(widget.id, body.current, target) : undefined, [runtime.maps, widget.id, target]);
  const state = useMapState(runtime.maps, widget.id);
  const desired = target(runtime.clock.getSnapshot());
  const matched = desired && state?.sourceId === desired.sourceId && !state.loading && !state.error && state.revision === desired.revision
    && state.branchId === desired.branchId && state.hour === desired.hour && state.sequence >= desired.sequence;
  const nativeUnavailable = !runtime.client.available || runtime.client.nativeMaps === false;
  return <div className="dash-map-widget">
    <div className="dash-map-surface" ref={body}>
      <div className="dash-map-placeholder" role={state?.error ? 'alert' : 'status'}><Icon name="map" />
        <strong>{nativeUnavailable ? 'Native map available in the desktop app' : runtime.nativeHidden ? 'Map hidden for layout controls'
          : state?.error ? 'This map could not be prepared' : !track ? 'Choose a source and track' : !desired ? 'No map at this time' : matched ? 'Native map' : 'Preparing this map…'}</strong>
        <p>{state?.error ?? (nativeUnavailable ? 'Open a recording in the desktop app to explore its campaign map.'
          : !desired && track ? 'The requested hour is outside this branch or its pin is unavailable.' : 'Follow the campaign as it unfolds, or pin a moment for comparison.')}</p>
        {state?.error && <button className="button small" type="button" onClick={() => runtime.maps.retry(widget.id)}>Retry map</button>}
      </div>
    </div>
    {runtime.client.nativeMaps !== false && state?.hour != null && <div className="dash-map-status">
      <span>{formatDateTime(state.hour)}</span>{!matched && <span className="is-pending">Updating…</span>}
    </div>}
  </div>;
}
