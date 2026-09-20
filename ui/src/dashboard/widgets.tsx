import { useCallback, useLayoutEffect, useMemo, useRef } from 'react';
import { formatDateTime } from '../analytics/calendar';
import type { MapOptions } from '../api';
import { Icon } from '../Icons';
import { resolveWindow } from './model';
import type { Alignment, BoundTrack, TrackSpec, WidgetSpec } from './model';
import { usePlaybackHour } from './playback';
import type { PlaybackController } from './playback';
import type { QueryStore } from './queries';
import { useMapState } from './geometry';
import type { MapTarget, MapViewsStore } from './geometry';
import type { DashboardClient } from './client';
import { resolveWidgetCampaign } from './campaigns';
import { widgetUnavailable } from './definitions';
import NationalFocusWidget from './NationalFocusWidget';

export interface WidgetRuntime {
  campaignTracks: TrackSpec[]; defaultRoleId: string | null; tracks: BoundTrack[]; clock: PlaybackController; queries: QueryStore; maps: MapViewsStore;
  onMapBranch: (trackId: string, branchId: string) => void;
  client: DashboardClient; alignment: Alignment; nativeHidden: boolean;
}
interface Props { widget: WidgetSpec; runtime: WidgetRuntime; active?: boolean; mapOptions?: MapOptions; headerSlot?: HTMLElement | null; onDiagnostics?: (warnings: string[]) => void }
const renderers: Readonly<Record<string, (props: Props) => React.ReactNode>> = { map: MapWidget, placeholder: Placeholder, 'national-focus': NationalFocusWidget };
export function WidgetBody({ widget, runtime, active = true, mapOptions, headerSlot, onDiagnostics }: Props) {
  const resolved = useMemo(() => resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId), [widget, runtime.campaignTracks, runtime.defaultRoleId]);
  const unavailable = widgetUnavailable(widget);
  const Renderer = Object.hasOwn(renderers, widget.kind) ? renderers[widget.kind] : undefined;
  if (unavailable || !Renderer) return <div className="dash-empty" role="status">{unavailable ?? 'Widget renderer unavailable.'}</div>;
  return <Renderer widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions} headerSlot={headerSlot} onDiagnostics={onDiagnostics} />;
}
function Placeholder() {
  return <div className="placeholder-panel"><span className="placeholder-cross" aria-hidden="true">+</span><span>Widget placeholder</span><small>Ready for your design</small></div>;
}

function MapWidget({ widget, runtime, active = true, mapOptions }: Props) {
  const body = useRef<HTMLDivElement>(null);
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find((item) => item.id === widget.trackIds[0]);
  const staticSequence = useMemo(() => runtime.clock.getSnapshot().sequence, [runtime.clock, widget, track]);
  const target = useCallback((clock: ReturnType<PlaybackController['getSnapshot']>): MapTarget | null => {
    if (!track) return null;
    const window = resolveWindow(widget, track, runtime.tracks, clock.position);
    if (!window || window.lastHour < track.branch.firstHour || window.lastHour > track.branch.lastHour) return null;
    return { options: { fog: mapOptions?.fog ?? true, country: widget.countries.kind === "tags" ? widget.countries.tags[0] ?? null : track.selectedTag }, sourceId: track.source.id, path: track.source.path, revision: track.source.revision, branchId: track.branch.id,
      hour: window.lastHour, fraction: widget.time.mode === 'follow' ? clock.fraction : 0,
      sequence: widget.time.mode === 'follow' ? clock.sequence : staticSequence };
  }, [track, widget, runtime.tracks, staticSequence, mapOptions]);
  useLayoutEffect(() => body.current ? runtime.maps.register(widget.id, body.current, target) : undefined, [runtime.maps, widget.id, target]);
  const state = useMapState(runtime.maps, widget.id);
  const desired = target(runtime.clock.getSnapshot());
  const matched = desired && state?.sourceId === desired.sourceId && !state.loading && !state.error && state.revision === desired.revision
    && state.branchId === desired.branchId && state.hour === desired.hour && state.sequence >= desired.sequence;
  return <div className="dash-map-widget">
    <div className="dash-map-surface" ref={body}>
      <div className="dash-map-placeholder" role={state?.error ? 'alert' : 'status'}><Icon name="map" />
        <strong>{(!runtime.client.available || runtime.client.nativeMaps === false) ? 'Native map available in the desktop app' : runtime.nativeHidden ? 'Map hidden for layout controls'
          : state?.error ? 'This map could not be prepared' : !track ? 'Choose a source and track' : !desired ? 'No map at this time' : matched ? 'Native map' : 'Preparing this map…'}</strong>
        <p>{state?.error ?? ((!runtime.client.available || runtime.client.nativeMaps === false) ? 'Open a recording in the desktop app to explore its campaign map.'
          : !desired && track ? 'The requested hour is outside this branch or its pin is unavailable.' : 'Follow the campaign as it unfolds, or pin a moment for comparison.')}</p>
        {state?.error && <button className="button small" type="button" onClick={() => runtime.maps.retry(widget.id)}>Retry map</button>}
      </div>
    </div>
    {runtime.client.nativeMaps !== false && state?.hour != null && <div className="dash-map-status">
      <span>{formatDateTime(state.hour)}</span>{!matched && <span className="is-pending">Updating…</span>}
    </div>}
  </div>;
}
