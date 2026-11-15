import { useCallback, useMemo } from 'react';
import type { CountryGroupReply, FactionDetailReply, FactionGroup } from '../generated/dashboard';
import type { BoundTrack } from './model';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { contentKey, setToken } from './options.ts';
import { selectedTags } from './countrySelection.ts';
import { factionNameAt as recordedNameAt } from './factionNames.ts';
import { headRead, useHeadProduct } from './headProduct.ts';
import { detailsStructure, validateFactionDetails } from './factionDetails.ts';
import { useCountryGroups } from './useCountryGroups';
import { useCampaignSettings } from './campaignSettings';
import { useHeadHour } from '../widgets/army-size/headHour.ts';

export { FACTION_OPTION } from './factionOption.ts';

const covers = (run: { firstHour: number; lastHour: number }, hour: number) => run.firstHour <= hour && hour <= run.lastHour;

/** The faction a country is a member of at `hour`: the recorded roster, never a guess. */
export function factionOfCountry(groups: CountryGroupReply, tag: string, hour: number): FactionGroup | null {
  return groups.factions.find(faction => faction.members.some(run => run.tag === tag && covers(run, hour))) ?? null;
}

export type NoFaction = 'none' | 'unobserved' | 'unknown' | 'absent';
export type FactionPick = { faction: FactionGroup; reason: null } | { faction: null; reason: NoFaction };

/** The saved faction while it exists (before it was founded or after it ended, it is absent), else the country's faction
 * at the hour; a gap in the faction rosters is not "in no faction". */
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

/** What a pick renders on: the faction, or why there is none. */
export const pickKey = (pick: FactionPick | null) => pick === null ? '' : pick.faction ? `faction:${pick.faction.id}` : `none:${pick.reason}`;

/** The faction's name as the game showed it at `hour`, never another hour's; without an
 * hour, its latest. The user's display name (`override`) applies throughout the campaign. */
export function factionNameAt(faction: FactionGroup, hour?: number, override?: string): string {
  return recordedNameAt(faction, hour, override).name ?? 'Faction name unavailable';
}

const NO_NAMES: Readonly<Record<string, string>> = Object.freeze({});

/** The display names the user gave this recording's campaign's factions, by faction id: every widget naming a faction
 * applies them, as the faction settings promise. */
export function useFactionDisplayNames(source: { metadata: { campaignKey: string } } | undefined): Readonly<Record<string, string>> {
  const styles = useCampaignSettings().campaigns[source?.metadata.campaignKey ?? '']?.factions;
  return useMemo(() => {
    if (!styles) return NO_NAMES;
    const named = Object.entries(styles).flatMap(([id, style]) => style.name ? [[id, style.name] as const] : []);
    return named.length ? Object.fromEntries(named) : NO_NAMES;
  }, [styles]);
}

/** Whether the faction's roster lists the country at `hour`. */
export const memberAt = (faction: FactionGroup, tag: string, hour: number) => faction.members.some(run => run.tag === tag && covers(run, hour));

/** The faction a widget shows, from its Faction setting and its country, and the track hour it was chosen at. While
 * following, the widget renders only when the faction (or why none is shown) changes, never because the head moved
 * inside it; pinned or on a range, at the chosen hour. Null hour outside the recorded branch. */
export function useChosenFaction(widget: WidgetSpec, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) {
  const groups = useCountryGroups(runtime, track, active);
  const saved = typeof widget.options?.faction === 'string' && widget.options.faction ? contentKey(widget.options.faction) : null;
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const { hourAt } = headRead(widget, track, runtime.tracks, null);
  const data = groups.data;
  const following = active && widget.time.mode === 'follow' && !!data;
  const key = useCallback((clockHour: number) => {
    const hour = hourAt(clockHour);
    return hour === null || !data ? 'outside' : pickKey(chosenFaction(data, saved, tag, hour));
  }, [hourAt, data, saved, tag]);
  const clockHour = useHeadHour(runtime.clock, following, key);
  const hour = hourAt(following ? clockHour : runtime.clock.getHour());
  const pick = useMemo(() => data && hour !== null ? chosenFaction(data, saved, tag, hour) : null, [data, saved, tag, hour]);
  return { groups, pick, tag, hour };
}

/** One faction's state at the head (a pushed panel while following; asked for the pinned hour otherwise). */
export function useFactionDetails(runtime: WidgetRuntime, widget: WidgetSpec, faction: string | null, active: boolean) {
  return useHeadProduct<FactionDetailReply>(runtime, widget, 'faction-details', faction ? { faction } : null,
    { active, validate: validateFactionDetails, structure: detailsStructure });
}

/** Every recorded faction, by its latest name, for the Faction setting. */
export function useFactionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, true);
  const names = useFactionDisplayNames(track?.source);
  return useMemo(() => ({ faction: (groups.data?.factions ?? []).map(faction => ({ value: setToken(faction.id),
    label: factionNameAt(faction, undefined, names[faction.id]) })) }), [groups.data, names]);
}

/** The state words every faction widget uses when no faction is shown. */
export function noFactionMessage(reason: NoFaction, live: boolean): string {
  return reason === 'absent' ? 'This faction did not exist at this time.'
    : reason === 'unobserved' ? 'Faction rosters were not recorded at this time.'
    : reason === 'unknown' ? 'This country’s faction is not known at this time.'
      : live ? 'This country is in no faction yet.' : 'This country is in no faction at this date.';
}
