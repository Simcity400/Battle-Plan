import { countryFromKey } from './countryKey.ts';
import { countryIdentityIndex } from './countryIdentity.ts';
import { useCallback, useMemo } from 'react';
import type { CountryGroupReply, FactionGroup } from '../generated/dashboard';
import type { BoundTrack } from './model';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { contentKey, setToken } from './options.ts';
import { selectedCountryKeys } from './countrySelection.ts';
import { factionNameAt as recordedNameAt } from './factionNames.ts';
import { headRead, useHeadProduct } from './headProduct.ts';
import { useGate } from './regionGate';
import { detailsStructure, validateFactionDetails } from './factionDetails.ts';
import { useCountryGroups } from './useCountryGroups';
import { useCampaignSettings } from './campaignSettings';
import { useHeadHour } from '../widgets/army-size/headHour.ts';
import { SPEC, axisAt } from './regionTime.ts';

export { FACTION_OPTION } from './factionOption.ts';

const covers = (run: { firstHour: number; lastHour: number }, hour: number) => run.firstHour <= hour && hour <= run.lastHour;

export function factionOfCountry(groups: CountryGroupReply, key: string, hour: number): FactionGroup | null {
  const index = countryIdentityIndex(groups.countryIdentities);
  const tags = new Set(typeof countryFromKey(key) === 'number' ? index.aliasesAt(index.selected([key]), hour) : [key]);
  return groups.factions.find(faction => faction.members.some(run => tags.has(run.tag) && covers(run, hour))) ?? null;
}

export type NoFaction = 'none' | 'unobserved' | 'unknown' | 'absent';
export type FactionPick = { faction: FactionGroup; reason: null } | { faction: null; reason: NoFaction };

export function chosenFaction(groups: CountryGroupReply, saved: string | null, tag: string | null, hour: number): FactionPick {
  if (saved) {
    const faction = groups.factions.find(item => item.id === saved);
    if (faction) return covers(faction, hour) ? { faction, reason: null } : { faction: null, reason: 'absent' };
  }
  if (groups.unobserved.factions.some(span => covers(span, hour))) return { faction: null, reason: 'unobserved' };
  if (!tag) return { faction: null, reason: 'none' };
  const faction = factionOfCountry(groups, tag, hour);
  if (faction) return { faction, reason: null };
  return { faction: null, reason: groups.factions.some(item => item.incomplete.some(span => covers(span, hour))) ? 'unknown' : 'none' };
}

export const pickKey = (pick: FactionPick | null) => pick === null ? '' : pick.faction ? `faction:${pick.faction.id}` : `none:${pick.reason}`;

export function factionNameAt(faction: FactionGroup, hour?: number, override?: string): string {
  return recordedNameAt(faction, hour, override).name ?? 'Faction name unavailable';
}

const NO_NAMES: Readonly<Record<string, string>> = Object.freeze({});

export function useFactionDisplayNames(source: { metadata: { campaignKey: string } } | undefined): Readonly<Record<string, string>> {
  const styles = useCampaignSettings().campaigns[source?.metadata.campaignKey ?? '']?.factions;
  return useMemo(() => {
    if (!styles) return NO_NAMES;
    const named = Object.entries(styles).flatMap(([id, style]) => style.name ? [[id, style.name] as const] : []);
    return named.length ? Object.fromEntries(named) : NO_NAMES;
  }, [styles]);
}

export const memberAt = (faction: FactionGroup, tag: string, hour: number) => faction.members.some(run => run.tag === tag && covers(run, hour));

export function useChosenFaction(widget: WidgetSpec, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) {
  const groups = useCountryGroups(runtime, track, active);
  const saved = typeof widget.options?.faction === 'string' && widget.options.faction ? contentKey(widget.options.faction) : null;
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const { hourAt } = headRead({ widget, action: SPEC }, track, runtime.tracks);
  const data = groups.data;
  const following = active && widget.time.mode === 'follow' && !!data;
  const key = useCallback((clockHour: number) => {
    const hour = hourAt(axisAt(clockHour));
    return hour === null || !data ? 'outside' : pickKey(chosenFaction(data, saved, tag, hour));
  }, [hourAt, data, saved, tag]);
  const clockHour = useHeadHour(runtime.clock, following, key);
  const hour = hourAt(axisAt(following ? clockHour : runtime.clock.getHour()));
  const pick = useMemo(() => data && hour !== null ? chosenFaction(data, saved, tag, hour) : null, [data, saved, tag, hour]);
  return { groups, pick, tag, hour };
}

export function useFactionDetails(runtime: WidgetRuntime, widget: WidgetSpec, faction: string | null, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const params = useMemo(() => faction ? { faction, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } : null, [faction, viewpointTag]);
  return useHeadProduct(runtime, useGate(runtime, widget), 'faction-details', params,
    { active, validate: validateFactionDetails, structure: detailsStructure });
}

export function useFactionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, true);
  const names = useFactionDisplayNames(track?.source);
  return useMemo(() => ({ faction: (groups.data?.factions ?? []).map(faction => ({ value: setToken(faction.id),
    label: factionNameAt(faction, undefined, names[faction.id]) })) }), [groups.data, names]);
}

export function noFactionMessage(reason: NoFaction, live: boolean): string {
  return reason === 'absent' ? 'This faction did not exist at this time.'
    : reason === 'unobserved' ? 'Faction rosters were not recorded at this time.'
    : reason === 'unknown' ? 'This country’s faction is not known at this time.'
      : live ? 'This country is in no faction yet.' : 'This country is in no faction at this date.';
}
