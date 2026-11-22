import { useMemo } from 'react';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { defaultCountryKey } from './countrySelection.ts';
import { worldFilterQuery, worldRequestTags, type WorldFilter } from './groups.ts';
import { useCountryGroups } from './useCountryGroups';
import { useAtHead } from './atHead.ts';
import { regionFollows, SPEC, type RegionAction, type TrackHour } from './regionTime.ts';
import { useRegion, useRegionHour, useRegionHourAt } from './widgetTime.ts';

const OUTSIDE = 'outside' as const;
type Read = typeof OUTSIDE | WorldFilter | null;
const sameFilter = (a: Read, b: Read) => a === b || typeof a === 'object' && typeof b === 'object' && a !== null && b !== null
  && a.tags === b.tags && a.reason === b.reason && a.known === b.known && a.countryIds === b.countryIds;

export function useCountriesSelectionAt(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, time: RegionAction = SPEC) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const needsGroups = widget.countries.kind !== 'all';
  const group = useCountryGroups(runtime, needsGroups ? track : undefined, active);
  const region = useRegion(widget, time);
  const hourAt = useRegionHourAt(runtime, region);
  const hour: TrackHour | null = useRegionHour(runtime, region, active);
  const tag = defaultCountryKey(track);
  const lastHour = track?.branch.lastHour ?? null;
  const ready = active && (!needsGroups || !!group.data);
  const query = useMemo(() => ready && lastHour !== null ? worldFilterQuery(widget.countries, tag ? [tag] : [], group.data ?? undefined, lastHour) : null,
    [ready, widget.countries, tag, group.data, lastHour]);
  const request = useMemo(() => ready && lastHour !== null ? worldRequestTags(widget.countries, tag ? [tag] : [], group.data ?? undefined, lastHour) : null,
    [ready, widget.countries, tag, group.data, lastHour]);
  const read = useAtHead(runtime.clock, active && regionFollows(region), clockHour => {
    const at = hourAt(clockHour);
    return at === null ? OUTSIDE : query ? query.at(at) : null;
  }, sameFilter);
  const filter = read === OUTSIDE ? null : read;
  return {
    track, group, hour: read === OUTSIDE ? null : hour, outside: read === OUTSIDE,
    countries: filter && !filter.reason ? filter.tags : undefined,
    requestTags: request?.ready ? request.tags.length ? request.tags : null : undefined,
    selectionReason: filter ? filter.reason ?? request?.reason ?? null : null,
    known: filter?.known ?? false,
  };
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
