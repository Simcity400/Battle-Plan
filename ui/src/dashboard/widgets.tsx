import { useEffect, useMemo } from 'react';
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
import { useExportSurface } from '../export/exportMotion';

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
  const exportSurface = useExportSurface();
  const resolved = useMemo(() => resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId), [widget, runtime.campaignTracks, runtime.defaultRoleId]);
  const unavailable = widgetUnavailable(widget);
  const definition = widgetDefinition(widget.kind);
  const Render = definition?.render;
  const time = useRegionScope(runtime, useRegion(resolved));
  const scopedTrack = !unavailable && definition && Render
    ? runtime.tracks.filter(track => resolved.trackIds.includes(track.id))
      .find(track => scopeReason(definition, track.source.metadata.captureMode) !== null)
    : undefined;
  const withheld = Boolean(unavailable || !definition || !Render || scopedTrack);
  useEffect(() => {
    if (!exportSurface.active || !exportSurface.reportWidget || !withheld) return undefined;
    const key = `${widget.id}:body`;
    exportSurface.reportWidget(key, {
      widgetId: widget.id,
      pipe: null,
      mode: 'unavailable',
      target: null,
      binding: 0,
      installedBinding: 0,
      installedId: null,
      expired: false,
      pendingIdentities: false,
      pendingArt: false,
    });
    return () => {
      exportSurface.reportWidget?.(key, null);
    };
  }, [exportSurface.active, exportSurface.reportWidget, withheld, widget.id]);
  if (unavailable || !definition || !Render) return <WidgetState kind="unavailable" message={unavailable ?? 'Widget renderer unavailable.'} />;
  if (scopedTrack) return <WidgetState kind="unavailable" cause={{ kind: 'capture-mode', mode: scopedTrack.source.metadata.captureMode }} />;
  return <RegionContext.Provider value={time}>
    <WidgetBoundary reset={resolved}>
      <Render widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null} filterSlot={filterSlot ?? null}
        onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />
    </WidgetBoundary>
  </RegionContext.Provider>;
}
