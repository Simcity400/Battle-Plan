import { useCallback, useLayoutEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { Icon } from '../../components/Icons';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { useMapState } from '../../dashboard/geometry';
import type { MapTarget } from '../../dashboard/geometry';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import type { PlaybackController } from '../../dashboard/playback';
import { mapPresentationMatches } from './presentation';

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
      seekSequence: widget.time.mode === 'follow' ? clock.seekSequence : undefined,
      playing: widget.time.mode === 'follow' && clock.playing,
      hoursPerSecond: clock.hoursPerSecond,
      hour: window.lastHour, fraction: widget.time.mode === 'follow' ? clock.fraction : 0,
      sequence: widget.time.mode === 'follow' ? clock.sequence : staticSequence };
  }, [track, widget, runtime.tracks, staticSequence, mapOptions]);
  useLayoutEffect(() => body.current ? runtime.maps.register(widget.id, body.current, target) : undefined, [runtime.maps, widget.id, target]);
  const state = useMapState(runtime.maps, widget.id);
  const desired = target(runtime.clock.getSnapshot());
  const matched = mapPresentationMatches(state, desired);
  const nativeUnavailable = !runtime.client.available || runtime.client.nativeMaps === false;
  return <div className="dash-map-widget">
    <div className="dash-map-surface" ref={body}>
      <div className="dash-map-placeholder" role={state?.error ? undefined : 'status'}>{state?.error
        ? <WidgetState kind="error" detail={state.error} onRetry={() => runtime.maps.retry(widget.id)} />
        : state?.pending
        ? <WidgetState kind="empty" icon="clock" message="Map not indexed yet" detail={state.pending} />
        : <><span className="empty-icon">{state?.loading || (desired && !matched && !nativeUnavailable && !runtime.nativeHidden) ? <span className="spinner" /> : <Icon name="map" />}</span>
        <strong>{nativeUnavailable ? 'Map available in the desktop app.' : runtime.nativeHidden ? 'Map hidden for layout controls'
          : !track ? 'Load a campaign' : !desired ? 'No map at this time' : matched ? 'Campaign map' : 'Preparing this map…'}</strong>
        <p>{nativeUnavailable ? 'Open a recording in the desktop app to explore its campaign map.'
          : !desired && track ? 'Choose another time.' : null}</p></>}
      </div>
    </div>
    {runtime.client.nativeMaps !== false && state?.hour != null && <div className="dash-map-status">
      <span><Icon name="clock" />{formatDateTime(state.hour)}</span>{matched ? <span className="pill positive">{widget.time.mode === 'follow' ? 'Live' : widget.time.mode === 'pin' ? 'Pinned' : 'Campaign end'}</span> : <span className="pill warning">Updating…</span>}
    </div>}
  </div>;
}
