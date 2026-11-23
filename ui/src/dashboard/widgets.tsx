import { useCallback, useMemo, useSyncExternalStore } from 'react';
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
import { RegionContext, useRegion, useRegionScope } from './widgetTime';

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
  const time = useRegionScope(runtime, useRegion(resolved));
  if (unavailable || !definition || !Render) return <div className="dash-empty" role="status">{unavailable ?? 'Widget renderer unavailable.'}</div>;
  if (playing && !definition.playsBackDated) return <div className="dash-empty" role="status">{widget.title} isn't playing back yet</div>;
  const scoped = runtime.tracks.filter(track => resolved.trackIds.includes(track.id))
    .map(track => scopeReason(definition, track.source.metadata.captureMode)).find(reason => reason !== null);
  if (scoped) return <WidgetState kind="unavailable" message={scoped} />;
  return <RegionContext.Provider value={time}>
    <WidgetBoundary reset={resolved}>
      <Render widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null}
        onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />
    </WidgetBoundary>
  </RegionContext.Provider>;
}
