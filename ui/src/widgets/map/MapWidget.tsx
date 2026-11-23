import { useCallback, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import { Icon } from '../../components/Icons';
import type { WidgetProps } from '../../dashboard/definitions';
import { useMapsHidden, useMapState } from '../../dashboard/geometry';
import type { MapTarget } from '../../dashboard/geometry';
import { resolveWindow } from '../../dashboard/model';
import type { PlaybackController } from '../../dashboard/playback';
import { mapFrameOnScreen, mapPresentationMatches } from './presentation';

export default function MapWidget({ widget, runtime, mapOptions }: WidgetProps) {
  const body = useRef<HTMLDivElement>(null);
  const anchor = useCallback(() => {
    const clock = runtime.clock.getSnapshot();
    return widget.time.mode === 'follow' ? `${clock.playing}:${clock.seekSequence}:${clock.playing ? '' : clock.hour}` : '';
  }, [runtime.clock, widget.time.mode]);
  useSyncExternalStore(runtime.clock.subscribe, anchor, anchor);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const staticSequence = useMemo(() => runtime.clock.getSnapshot().sequence, [runtime.clock, widget, track]);
  const target = useCallback((clock: ReturnType<PlaybackController['getSnapshot']>): MapTarget | null => {
    if (!track) return null;
    const window = resolveWindow(widget, track, runtime.tracks, clock.position);
    if (!window || window.lastHour < track.branch.firstHour || window.lastHour > track.branch.lastHour) return null;
    const pinned = widget.countries.kind === 'tags';
    const country = widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : track.selectedTag;
    return { options: { fog: mapOptions.fog, country, countrySlot: pinned ? null : track.selectedSlot ?? null, countryPinned: pinned,
      observed: pinned ? null : track.observed?.[widget.id] ?? null }, sourceId: track.source.id, path: track.source.path, revision: track.source.revision, branchId: track.branch.id,
      seekSequence: widget.time.mode === 'follow' ? clock.seekSequence : undefined,
      playing: widget.time.mode === 'follow' && clock.playing,
      hoursPerSecond: clock.hoursPerSecond,
      hour: window.lastHour, fraction: widget.time.mode === 'follow' ? clock.fraction : 0,
      sequence: widget.time.mode === 'follow' ? clock.sequence : staticSequence };
  }, [track, widget, runtime.tracks, staticSequence, mapOptions]);
  useLayoutEffect(() => body.current ? runtime.maps.register(widget.id, body.current, target) : undefined, [runtime.maps, widget.id, target]);
  const state = useMapState(runtime.maps, widget.id);
  const hidden = useMapsHidden(runtime.maps);
  const desired = target(runtime.clock.getSnapshot());
  const matched = mapPresentationMatches(state, desired);
  const nativeUnavailable = !runtime.client.available || runtime.client.nativeMaps === false;
  const framed = !nativeUnavailable && mapFrameOnScreen(state);
  const held = framed && !matched && !hidden && !state?.error ? (desired?.playing ? 'behind' : 'stale') : undefined;
  const preparing = !hidden && !framed && (state?.loading || (desired && !matched && !nativeUnavailable));
  return <div className="dash-map-widget" data-map-current={matched} data-held={held}>
    <div className="dash-map-surface" ref={body}>
      <div className="dash-map-placeholder" role={state?.error ? undefined : 'status'}>{state?.error
        ? <WidgetState kind="error" what="the campaign map" failure={new Error(state.error)} onRetry={() => runtime.maps.retry(widget.id)} />
        : !desired && track && !nativeUnavailable && !runtime.nativeHidden && !hidden
          ? <WidgetState kind="outside" />
          : <><span className="empty-icon">{preparing ? <span className="spinner" /> : <Icon name="map" />}</span>
          <strong>{nativeUnavailable ? 'Map available in the desktop app.' : runtime.nativeHidden ? 'Map hidden for layout controls.'
            : hidden ? 'Map hidden while a menu is open.'
            : !track ? 'Load a campaign to see the map.' : matched || framed ? 'Campaign map' : 'Preparing this map…'}</strong>
          {nativeUnavailable && <p>Open a recording in the desktop app to explore its campaign map.</p>}</>}
      </div>
    </div>
  </div>;
}
