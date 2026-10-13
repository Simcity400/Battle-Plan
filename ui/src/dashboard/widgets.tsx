import { useMemo } from 'react';
import type { MapOptions } from '../shell/api';
import type { WidgetSpec } from './model';
import { resolveWidgetCampaign } from './campaigns';
import { widgetDefinition, widgetUnavailable } from './definitions';
import type { WidgetRuntime } from './definitions';
import type { OptionValue } from './options';
import WidgetBoundary from './WidgetBoundary';

export type { WidgetRuntime } from './definitions';

interface Props { widget: WidgetSpec; runtime: WidgetRuntime; active?: boolean; mapOptions?: MapOptions; headerSlot?: HTMLElement | null; countrySlot?: HTMLElement | null; onDiagnostics?: (warnings: string[]) => void;
  setOption?: (key: string, value: OptionValue) => void; setOptions?: (values: Readonly<Record<string, OptionValue>>) => void }
const DEFAULT_MAP_OPTIONS: MapOptions = { country: null, fog: true };
const noDiagnostics = () => {};

/** Renders the registered widget for a spec; the grid never switches on a kind. */
export function WidgetBody({ widget, runtime, active = true, mapOptions, headerSlot, countrySlot, onDiagnostics, setOption, setOptions }: Props) {
  const resolved = useMemo(() => resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId), [widget, runtime.campaignTracks, runtime.defaultRoleId]);
  const unavailable = widgetUnavailable(widget);
  const Render = widgetDefinition(widget.kind)?.render;
  if (unavailable || !Render) return <div className="dash-empty" role="status">{unavailable ?? 'Widget renderer unavailable.'}</div>;
  return <WidgetBoundary reset={resolved}>
    <Render widget={resolved} runtime={runtime} active={active} mapOptions={mapOptions ?? DEFAULT_MAP_OPTIONS} headerSlot={headerSlot ?? null} countrySlot={countrySlot ?? null}
      onDiagnostics={onDiagnostics ?? noDiagnostics} setOption={setOption} setOptions={setOptions} />
  </WidgetBoundary>;
}
