import { clipToSpans, isOrderedSpans, runAt } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { countryDrillSpans, historyAt } from '../../dashboard/countryIdentity.ts';
import type { BoundTrack } from '../../dashboard/model';
import { signed32 } from '../../dashboard/validation.ts';
import type { CountryIdentityHistory } from '../../generated/dashboard';

export interface DrillCountry {
  campaign: string; branch: string; tag: string; originOffset: number; slot: number; countryId: number;
}

export function chooseDrillCountry(histories: readonly CountryIdentityHistory[], track: BoundTrack, tag: string, hour: number): DrillCountry | null {
  const origin = Math.floor(hour);
  const history = historyAt(histories, tag, origin);
  const identity = runAt(history?.identities ?? [], origin);
  return history && identity && origin >= track.branch.firstHour && origin <= track.branch.lastHour
    ? { campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag: history.tag,
      originOffset: origin - track.branch.firstHour, slot: identity.slot, countryId: identity.countryId } : null;
}

export function drillCountrySpans(histories: readonly CountryIdentityHistory[], pick: DrillCountry,
  track: BoundTrack, membership?: ReadonlyMap<string, readonly Span[]> | null): ReadonlyMap<string, readonly Span[]> | null {
  const origin = track.branch.firstHour + pick.originOffset;
  if (!signed32(origin) || origin < track.branch.firstHour || origin > track.branch.lastHour) return null;
  const witnessed = runAt(histories.find(history => history.tag === pick.tag)?.identities ?? [], origin);
  if (!witnessed || witnessed.slot !== pick.slot || witnessed.countryId !== pick.countryId) return null;
  const aliases = countryDrillSpans(histories, pick.tag, origin);
  if (!isOrderedSpans([...aliases.values()].flat().sort((a, b) => a.firstHour - b.firstHour))) return null;
  return new Map([...aliases].filter(([, spans]) => spans.length).map(([tag, spans]) => [tag,
    membership ? clipToSpans(spans, membership.get(tag) ?? []) : spans]));
}

export function drillCountryCoverage(spans: ReadonlyMap<string, readonly Span[]>): Span[] {
  const coverage: Span[] = [];
  for (const span of [...spans.values()].flat().sort((a, b) => a.firstHour - b.firstHour)) {
    const previous = coverage.at(-1);
    if (previous && previous.lastHour + 1 === span.firstHour) previous.lastHour = span.lastHour;
    else coverage.push({ firstHour: span.firstHour, lastHour: span.lastHour });
  }
  return coverage;
}
