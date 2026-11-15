import { useMemo } from 'react';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { defaultCountryTag } from './countrySelection.ts';
import { worldFilter, worldRequestTags } from './groups.ts';
import { useCountryGroups } from './useCountryGroups';

export function useCountriesSelectionAt(widget: WidgetSpec, runtime: WidgetRuntime, hour: number, active: boolean) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const needsGroups = widget.countries.kind !== 'all';
  const group = useCountryGroups(runtime, needsGroups ? track : undefined, active);
  const selected = useMemo(() => {
    const tag = defaultCountryTag(track);
    const fallback = tag ? [tag] : [];
    const lastHour = track?.branch.lastHour ?? hour;
    const filter = worldFilter(widget.countries, fallback, group.data ?? undefined, hour, lastHour);
    const request = worldRequestTags(widget.countries, fallback, group.data ?? undefined, lastHour);
    const ready = !needsGroups || !!group.data;
    return {
      countries: ready && !filter.reason ? filter.tags : undefined,
      requestTags: request.ready ? request.tags.length ? request.tags : null : undefined,
      selectionReason: filter.reason ?? request.reason,
      // False while the countries at `hour` are not yet read: no row is not yet proof of none.
      known: filter.known,
    };
  }, [widget.countries, track, group.data, hour, needsGroups]);
  return { track, group, ...selected };
}
