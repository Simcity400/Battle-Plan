import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import type { TerritoryMeasure, TerritoryParams, TerritoryReply, TerritoryRule } from '../../generated/dashboard';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import type { ResolvedCountries } from '../../dashboard/groups';

export { SMOOTH_TOTALS_PRESENTATION as TERRITORY_PRESENTATION } from '../../charts/race/factionRaceData.ts';
export { MEASURES, measureOf, ruleOf, measureChoices, formatTerritory, territoryNote, validateTerritoryReply, territoryColumn } from '../../dashboard/territoryMeasure.ts';

import { territoryColumn } from '../../dashboard/territoryMeasure.ts';

export function territoryRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, measure: TerritoryMeasure, rule: TerritoryRule): TerritoryParams {
  territoryColumn(measure, rule);
  return { branchId, tags: resolved.top != null ? [] : [...resolved.tags], top: resolved.top != null ? { count: resolved.top, measure, rule } : null };
}

/** Prepare the selected measure once per reply. Null columns split only their
 * own line, so a failed province read does not erase a recorded state count. */
export function territoryModel(reply: TerritoryReply, measure: TerritoryMeasure, rule: TerritoryRule, chosen?: readonly string[]) {
  const column = territoryColumn(measure, rule);
  const picked = chosen && new Set(chosen);
  const selected = new Set(reply.series.filter(series => picked?.has(series.tag))
    .flatMap(series => (series.identities ?? []).map(countryIdentityKey)));
  const series = new Map(reply.series.map(series => {
    const runs: Run[] = [];
    for (const run of series.runs) {
      const value = run.values[column];
      if (value === null) continue;
      const previous = runs.at(-1);
      if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.value === value) previous.lastHour = run.lastHour;
      else runs.push({ firstHour: run.firstHour, lastHour: run.lastHour, value, ...(run.startsSegment ? { breakBefore: true } : {}) });
    }
    const identities = picked && series.identities ? series.identities.filter(identity => selected.has(countryIdentityKey(identity))) : series.identities;
    const segments = picked ? identities ?? (picked.has(series.tag) ? series.segments : []) : series.segments;
    return [series.tag, { runs, segments, membership: series.membership, identities }] as const;
  }));
  return { series, tags: [...series.keys()], manual: picked !== undefined, selectedCount: Math.max(picked?.size ?? 0, selected.size) };
}

/** Absence is a zero witness for a fixed group sum, not an individual country line. */
export function territoryCountryLines(model: ReturnType<typeof territoryModel>, resolved: ResolvedCountries, range: Span, includeAbsent = false) {
  const tags = model.manual || resolved.top != null ? model.tags : resolved.tags;
  return tags.flatMap(tag => {
    const series = model.series.get(tag);
    if (!series) return [];
    const runs = shownTerritory(includeAbsent ? series.runs : clipToSpans(series.runs, series.segments),
      resolved.membership?.get(tag) ?? null, range);
    return [{ tag, runs }];
  });
}

/** A present selected country with a missing measure still refuses the whole share. */
export function territoryShareKeys(model: ReturnType<typeof territoryModel>, resolved: ResolvedCountries, hour: number): string[] {
  const tags = model.manual || resolved.top != null ? model.tags : resolved.tags;
  return tags.filter(tag => runAt(model.series.get(tag)?.segments ?? [], hour)
    && (!resolved.membership || runAt(resolved.membership.get(tag) ?? [], hour)));
}

export function shownTerritory(runs: readonly Run[], membership: readonly Span[] | null, range: Span): Run[] {
  return clipToSpans(membership ? clipToSpans(runs, membership) : runs, [range]);
}

export function territoryLegend(factions: boolean) {
  return { label: factions ? 'Factions' : 'Countries' };
}

export function territorySegmentAt(model: ReturnType<typeof territoryModel> | null, tag: string, hour: number): number | null {
  return runAt(model?.series.get(tag)?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
}

/** Area availability affects diagnostics only when that measure is drawn. */
export function territoryWarnings(reply: Pick<TerritoryReply, 'warnings' | 'areaReason'> | undefined, measure: TerritoryMeasure): string[] {
  return [...(reply?.warnings ?? []), ...(measure === 'area' && reply?.areaReason ? [reply.areaReason] : [])];
}
