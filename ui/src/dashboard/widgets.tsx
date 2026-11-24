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
import DeclaredTableWidget from '../families/table/DeclaredTableWidget';
import DeclaredTimelineWidget from '../families/timeline/DeclaredTimelineWidget';

export type { WidgetRuntime } from './definitions';

interface Props {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  active?: boolean;
  mapOptions?: MapOptions;
  headerSlot?: HTMLElement | null;
  countrySlot?: HTMLElement | null;
  onDiagnostics?: (warnings: string[]) => void;
  setOption?: (key: string, value: OptionValue) => void;
  setOptions?: (values: Readonly<Record<string, OptionValue>>) => void;
}
const noDiagnostics = () => {};
const MEASURE_UNGATED = typeof __BATTLEPLAN_MEASURE_UNGATED__ !== 'undefined' && __BATTLEPLAN_MEASURE_UNGATED__;

export function WidgetBody({ widget, runtime, active = true, mapOptions, headerSlot, countrySlot, onDiagnostics, setOption, setOptions }: Props) {
  const resolved = useMemo(() => resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId), [widget, runtime.campaignTracks, runtime.defaultRoleId]);
  const unavailable = widgetUnavailable(widget);
  const definition = widgetDefinition(widget.kind);
  const Render = definition?.render;
  const declaration = definition?.declaration;
  const readPlaying = useCallback(() => runtime.clock.getSnapshot().playing, [runtime.clock]);
  const playing = useSyncExternalStore(runtime.clock.subscribe, readPlaying, readPlaying);
  const time = useRegionScope(runtime, useRegion(resolved));
  if (unavailable || !definition || (!Render && !declaration)) return <WidgetState kind="unavailable" message={unavailable ?? 'Widget renderer unavailable.'} />;
  if (!MEASURE_UNGATED && playing && !definition.playsBackDated) return <div className="widget-state dash-empty" data-state="placeholder" data-playback-placeholder="" role="status"><span className="widget-state-title">{widget.title} isn’t playing back yet.</span></div>;
  const scopedTrack = runtime.tracks.filter(track => resolved.trackIds.includes(track.id))
    .find(track => scopeReason(definition, track.source.metadata.captureMode) !== null);
  if (scopedTrack) return <WidgetState kind="unavailable" cause={{ kind: 'capture-mode', mode: scopedTrack.source.metadata.captureMode }} />;
  return <RegionContext.Provider value={time}>
    <WidgetBoundary reset={resolved}>
      {Render
        ? <Render widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null}
            onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />
        : declaration!.family === 'timeline'
          ? <DeclaredTimelineWidget declaration={declaration!} widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null}
              onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />
          : <DeclaredTableWidget declaration={declaration!} widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null}
              onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />}
    </WidgetBoundary>
  </RegionContext.Provider>;
}
