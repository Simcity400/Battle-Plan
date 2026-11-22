import { clipToSpans, isOrderedSpans, runAt } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { countryDrillSpans, historyAt } from '../../dashboard/countryIdentity.ts';
import type { BoundTrack, CountrySelection } from '../../dashboard/model';
import { object, opaqueTag, signed32 } from '../../dashboard/validation.ts';
import type { CountryIdentityHistory } from '../../generated/dashboard';
import type { HeldCountry } from '../../dashboard/regionGate';

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

export const heldDrillCountry = (held: HeldCountry, track: BoundTrack): DrillCountry => ({ campaign: track.source.metadata.campaignKey, branch: track.branch.id,
  tag: held.tag, originOffset: held.origin - track.branch.firstHour, slot: held.slot, countryId: held.countryId });

export function readDrillCountry(value: unknown, track: BoundTrack | undefined, countries: CountrySelection): DrillCountry | null {
  if (typeof value !== 'string' || !track) return null;
  try {
    const pick: unknown = JSON.parse(value);
    return object(pick) && pick.campaign === track.source.metadata.campaignKey && pick.branch === track.branch.id
      && opaqueTag(pick.tag) && countries.kind === 'tags' && countries.tags.length === 1 && countries.tags[0] === pick.tag
      && Number.isSafeInteger(pick.originOffset) && typeof pick.originOffset === 'number' && pick.originOffset >= 0
      && [pick.slot, pick.countryId].every(value => typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff)
      ? pick as unknown as DrillCountry : null;
  } catch { return null; }
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
