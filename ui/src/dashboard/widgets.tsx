import { useMemo } from 'react';
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

interface Props {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  active?: boolean;
  mapOptions?: MapOptions;
  headerSlot?: HTMLElement | null;
  countrySlot?: HTMLElement | null;
  filterSlot?: HTMLElement | null;
  onDiagnostics?: (warnings: string[]) => void;
  setOption?: (key: string, value: OptionValue) => void;
  setOptions?: (values: Readonly<Record<string, OptionValue>>) => void;
}
const noDiagnostics = () => {};

export function WidgetBody({ widget, runtime, active = true, mapOptions, headerSlot, countrySlot, filterSlot, onDiagnostics, setOption, setOptions }: Props) {
  const resolved = useMemo(() => resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId), [widget, runtime.campaignTracks, runtime.defaultRoleId]);
  const unavailable = widgetUnavailable(widget);
  const definition = widgetDefinition(widget.kind);
  const Render = definition?.render;
  const time = useRegionScope(runtime, useRegion(resolved));
  if (unavailable || !definition || !Render) return <WidgetState kind="unavailable" message={unavailable ?? 'Widget renderer unavailable.'} />;
  const scopedTrack = runtime.tracks.filter(track => resolved.trackIds.includes(track.id))
    .find(track => scopeReason(definition, track.source.metadata.captureMode) !== null);
  if (scopedTrack) return <WidgetState kind="unavailable" cause={{ kind: 'capture-mode', mode: scopedTrack.source.metadata.captureMode }} />;
  return <RegionContext.Provider value={time}>
    <WidgetBoundary reset={resolved}>
      <Render widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null} filterSlot={filterSlot ?? null}
        onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />
    </WidgetBoundary>
  </RegionContext.Provider>;
}
