import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { SPEC, type RegionAction } from './regionTime.ts';
import { useRegion } from './widgetTime.ts';
import { useRegionGate } from './regionGate.ts';

export function useCountriesSelectionAt(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, time: RegionAction = SPEC) {
  const gate = useRegionGate(runtime, useRegion(widget, time), active, { members: true });
  const members = gate.members!;
  return { gate, track: gate.track, group: gate.groups, hour: gate.hour, outside: gate.outside !== null,
    countries: members.countries, requestTags: members.requestTags, selectionReason: members.selectionReason, known: members.known };
}
