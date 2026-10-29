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
  const manual = selection.kind === 'tags' || selection.kind === 'selected';
  // Chosen countries need the identity history too: each native holder follows its renames.
  const groups = useCountryGroups(runtime, track, active && (grouped || manual));
  const lastHour = track?.branch.lastHour ?? 0;
  // Pending or empty choices never ask for the most active countries instead.
  const roots = grouped ? worldRequestTags(selection, [], groups.data, lastHour) : null;
  const chosen = manual ? selectedTags(selection, track) : roots?.tags;
  const chosenKey = JSON.stringify(chosen ?? null);
  // Every tag the chosen native holders held, with when: each is found under the one it has at the hour shown.
  const aliases = useMemo(() => manual && chosen?.length && groups.data
    ? countryMatchFilter({ kind: 'tags', tags: chosen }, [], groups.data, lastHour).membership : null,
  [manual, chosenKey, groups.data, lastHour]); // eslint-disable-line react-hooks/exhaustive-deps
  // Members are asked for under every tag they held, so each is found under the one it has at the hour shown.
  const held = useMemo(() => grouped && roots?.ready
    ? [...new Set(countryMatchFilter(selection, [], groups.data, lastHour).membership?.map(run => run.tag) ?? [])].sort()
    : aliases ? [...new Set(aliases.map(run => run.tag))].sort() : undefined,
  [grouped, roots?.ready, selection, groups.data, lastHour, aliases]);
  // Without the identity history (still loading) chosen tags are not asked for yet; if it fails, by their literal tags.
  const waiting = manual && !groups.data && !groups.error;
  const request = roots && !roots.ready || chosen?.length === 0 || waiting ? null : intelCoverageRequest(widget, track, chosen, held);
  const product = useProduct<IntelCoverageReply>(runtime.client, request, active, validateIntelCoverage, track?.source);
  return { track, options, grouped, groups, roots, chosen, aliases, touching: intelTouching(widget, chosen), ...product };
}
