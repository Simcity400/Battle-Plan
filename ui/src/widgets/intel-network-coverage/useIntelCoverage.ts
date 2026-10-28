import type { WidgetRuntime } from '../../dashboard/definitions';
import { selectedTags } from '../../dashboard/countrySelection';
import { worldRequestTags } from '../../dashboard/groups';
import { intelCoverageRequest } from '../../dashboard/widgetRequests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { validateIntelCoverage } from './intelCoverageData';
import type { IntelCoverageReply } from './intelCoverageData';
import { OPTIONS } from './definition';

export const useIntelOptions = (widget: WidgetSpec) => resolveOptions(OPTIONS, widget.options);

/** The shown countries' networks over the branch: the chosen countries (the dashboard's by
 * default), a group's members over the whole branch, or else the most active, looked up
 * locally at every hour. */
export function useIntelCoverage(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = useIntelOptions(widget);
  const selection = widget.countries;
  const grouped = selection.kind === 'group' && selection.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  // Pending or empty choices never ask for the most active countries instead.
  const roots = grouped ? worldRequestTags(selection, [], groups.data, track?.branch.lastHour ?? 0) : null;
  const chosen = selection.kind === 'tags' || selection.kind === 'selected' ? selectedTags(selection, track) : roots?.tags;
  const request = roots && !roots.ready || chosen?.length === 0 ? null : intelCoverageRequest(widget, track, chosen);
  const product = useProduct<IntelCoverageReply>(runtime.client, request, active, validateIntelCoverage, track?.source);
  return { track, options, grouped, groups, roots, chosen, ...product };
}
