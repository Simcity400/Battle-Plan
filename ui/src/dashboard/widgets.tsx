import { useCallback, useMemo, useState, useSyncExternalStore } from 'react';
import type { MapOptions } from '../shell/api';
import type { WidgetSpec } from './model';
import { resolveWidgetCampaign } from './campaigns';
import { widgetDefinition, widgetUnavailable } from './definitions';
import type { WidgetRuntime } from './definitions';
import type { OptionValue } from './options';
import WidgetBoundary from './WidgetBoundary';
import WidgetState from '../components/WidgetState';
import { scopeReason } from './widgetAvailability';
import { DEFAULT_MAP_OPTIONS } from './mapPicker';
import { ActiveHeads, WidgetTimeContext } from './widgetTime';
import { headRead, useOutsideTime } from './headProduct';

export type { WidgetRuntime } from './definitions';

interface Props { widget: WidgetSpec; runtime: WidgetRuntime; active?: boolean; mapOptions?: MapOptions; headerSlot?: HTMLElement | null; countrySlot?: HTMLElement | null; onDiagnostics?: (warnings: string[]) => void;
  setOption?: (key: string, value: OptionValue) => void; setOptions?: (values: Readonly<Record<string, OptionValue>>) => void }
const noDiagnostics = () => {};

export function WidgetBody({ widget, runtime, active = true, mapOptions, headerSlot, countrySlot, onDiagnostics, setOption, setOptions }: Props) {
  const resolved = useMemo(() => resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId), [widget, runtime.campaignTracks, runtime.defaultRoleId]);
  const unavailable = widgetUnavailable(widget);
  const definition = widgetDefinition(widget.kind);
  const Render = definition?.render;
  const readPlaying = useCallback(() => runtime.clock.getSnapshot().playing, [runtime.clock]);
  const playing = useSyncExternalStore(runtime.clock.subscribe, readPlaying, readPlaying);
  const track = runtime.tracks.find(item => item.id === resolved.trackIds[0]);
  const { hourAt } = useMemo(() => headRead(resolved, track, runtime.tracks, null), [resolved, track, runtime.tracks]);
  const outside = useOutsideTime(runtime.clock, !!track, hourAt);
  const [heads] = useState(() => new ActiveHeads());
  const pinned = resolved.time.mode !== 'follow';
  const time = useMemo(() => ({ heads, outside, detail: pinned ? 'Choose another time in widget settings.' : undefined }), [heads, outside, pinned]);
  if (unavailable || !definition || !Render) return <div className="dash-empty" role="status">{unavailable ?? 'Widget renderer unavailable.'}</div>;
  if (playing && !definition.playsBackDated) return <div className="dash-empty" role="status">{widget.title} isn't playing back yet</div>;
  const scoped = runtime.tracks.filter(track => resolved.trackIds.includes(track.id))
    .map(track => scopeReason(definition, track.source.metadata.captureMode)).find(reason => reason !== null);
  if (scoped) return <WidgetState kind="unavailable" message={scoped} />;
  return <WidgetTimeContext.Provider value={time}>
    <WidgetBoundary reset={resolved}>
      <Render widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null}
        onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />
    </WidgetBoundary>
  </WidgetTimeContext.Provider>;
}
