import { useMemo } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { selectedTags } from '../../dashboard/countrySelection';
import { countryMatchFilter, worldRequestTags } from '../../dashboard/groups';
import { intelCoverageRequest, intelTouching } from '../../dashboard/widgetRequests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { validateIntelCoverage } from './intelCoverageData';
import type { IntelCoverageReply } from './intelCoverageData';
import { OPTIONS } from './definition';

export const useIntelOptions = (widget: WidgetSpec) => resolveOptions(OPTIONS, widget.options);

/** The shown countries' networks over the branch: the chosen countries (the dashboard's by
 * default), a group's members over the whole branch, or else the most active, looked up locally
 * at every hour. One country alone, chosen or the top one, comes with every country it has a
 * network in or that has one in it (`intelTouching`). */
export function useIntelCoverage(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = useIntelOptions(widget);
  const selection = widget.countries;
  const grouped = selection.kind === 'group' && selection.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const lastHour = track?.branch.lastHour ?? 0;
  // Pending or empty choices never ask for the most active countries instead.
  const roots = grouped ? worldRequestTags(selection, [], groups.data, lastHour) : null;
  const chosen = selection.kind === 'tags' || selection.kind === 'selected' ? selectedTags(selection, track) : roots?.tags;
  // Members are asked for under every tag they held, so each is found under the one it has at the hour shown.
  const held = useMemo(() => grouped && roots?.ready
    ? [...new Set(countryMatchFilter(selection, [], groups.data, lastHour).membership?.map(run => run.tag) ?? [])].sort() : undefined,
  [grouped, roots?.ready, selection, groups.data, lastHour]);
  const request = roots && !roots.ready || chosen?.length === 0 ? null : intelCoverageRequest(widget, track, held ?? chosen);
  const product = useProduct<IntelCoverageReply>(runtime.client, request, active, validateIntelCoverage, track?.source);
  return { track, options, grouped, groups, roots, chosen, touching: intelTouching(widget, chosen), ...product };
}
