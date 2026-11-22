import { useMemo } from 'react';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { defaultCountryKey } from './countrySelection.ts';
import { worldRequestTags } from './groups.ts';
import { useCountryGroups } from './useCountryGroups';
import { SPEC, type RegionAction } from './regionTime.ts';
import { useRegion } from './widgetTime.ts';
import { useRegionGate } from './regionGate.ts';

export function useCountriesSelectionAt(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, time: RegionAction = SPEC) {
  const gate = useRegionGate(runtime, useRegion(widget, time), active, { members: true });
  const members = gate.members!;
  return { gate, track: gate.track, group: gate.groups, hour: gate.hour, outside: gate.outside !== null,
    countries: members.countries, requestTags: members.requestTags, selectionReason: members.selectionReason, known: members.known };
}

export function useCountriesRequest(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const needsGroups = widget.countries.kind !== 'all';
  const group = useCountryGroups(runtime, needsGroups ? track : undefined, active);
  const tag = defaultCountryKey(track);
  const ready = !needsGroups || !!group.data;
  const request = useMemo(() => ready && track ? worldRequestTags(widget.countries, tag ? [tag] : [], group.data ?? undefined, track.branch.lastHour) : null,
    [ready, widget.countries, tag, group.data, track]);
  return { track, group, requestTags: request?.ready ? request.tags.length ? request.tags : null : undefined, selectionReason: request?.reason ?? null };
}
