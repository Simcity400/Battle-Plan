import { useCallback, useMemo } from 'react';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { SPEC, type RegionAction } from './regionTime.ts';
import { useRegion } from './widgetTime.ts';
import { useRegionGate } from './regionGate.ts';
import { defaultCountryKey } from './countrySelection.ts';
import { worldFilterQuery } from './groups.ts';

export function useCountriesSelectionAt(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, time: RegionAction = SPEC) {
  const gate = useRegionGate(runtime, useRegion(widget, time), active, { members: true });
  const members = gate.members!;
  const ranked = widget.countries.kind === 'all' || widget.countries.kind === 'group' && widget.countries.group.type === 'top';
  const track = gate.track, groups = gate.groups.data;
  const query = useMemo(() => !ranked && groups && track ? worldFilterQuery(widget.countries, [defaultCountryKey(track)].flatMap(tag => tag ? [tag] : []), groups, track.branch.lastHour) : null,
    [ranked, groups, track, widget.countries]);
  const countriesAt = useCallback((hour: number): ReadonlySet<string> | null | undefined => {
    if (ranked) return null;
    const filter = query?.at(hour);
    return filter && !filter.reason ? filter.tags : undefined;
  }, [ranked, query]);
  return { gate, track, group: gate.groups, hour: gate.hour, outside: gate.outside !== null,
    countries: members.countries, countriesAt, requestTags: members.requestTags,
    selectionReason: gate.stage.stage === 'no-members' ? gate.stage.reason : members.selectionReason, known: members.known };
}
