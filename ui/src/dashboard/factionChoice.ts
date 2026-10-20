import { useMemo } from 'react';
import type { CountryGroupReply, FactionDetailReply, FactionGroup } from '../generated/dashboard';
import type { BoundTrack } from './model';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { contentKey, setToken } from './options.ts';
import { selectedTags } from './countrySelection.ts';
import { useProduct } from './products';
import { useCountryGroups } from './useCountryGroups';

export { FACTION_OPTION } from './factionOption.ts';

const covers = (run: { firstHour: number; lastHour: number }, hour: number) => run.firstHour <= hour && hour <= run.lastHour;

/** The faction a country is a member of at `hour`: the recorded roster, never a guess. */
export function factionOfCountry(groups: CountryGroupReply, tag: string, hour: number): FactionGroup | null {
  return groups.factions.find(faction => faction.members.some(run => run.tag === tag && covers(run, hour))) ?? null;
}

export type FactionPick = { faction: FactionGroup; reason: null } | { faction: null; reason: 'none' | 'unobserved' | 'unknown' };

/** The saved faction while it exists, else the country's faction at the hour; a gap in
 * the faction rosters is not "in no faction". */
export function chosenFaction(groups: CountryGroupReply, saved: string | null, tag: string | null, hour: number): FactionPick {
  if (saved) {
    const faction = groups.factions.find(item => item.id === saved);
    if (faction) return { faction, reason: null };
  }
  if (groups.unobserved.factions.some(span => covers(span, hour))) return { faction: null, reason: 'unobserved' };
  if (!tag) return { faction: null, reason: 'none' };
  const faction = factionOfCountry(groups, tag, hour);
  if (faction) return { faction, reason: null };
  return { faction: null, reason: groups.factions.some(item => item.incomplete.some(span => covers(span, hour))) ? 'unknown' : 'none' };
}

/** The faction's name as the game showed it at `hour` (its latest outside its rows). */
export function factionNameAt(faction: FactionGroup, hour: number): string {
  const run = faction.names.find(item => covers(item, hour)) ?? faction.names.at(-1);
  return run?.name ?? faction.name ?? 'Faction name unavailable';
}

/** Whether the faction's roster lists the country at `hour`. */
export const memberAt = (faction: FactionGroup, tag: string, hour: number) => faction.members.some(run => run.tag === tag && covers(run, hour));

/** The faction a widget shows, from its Faction setting and its country. */
export function useChosenFaction(widget: WidgetSpec, runtime: WidgetRuntime, track: BoundTrack | undefined, hour: number, active: boolean) {
  const groups = useCountryGroups(runtime, track, active);
  const saved = typeof widget.options?.faction === 'string' && widget.options.faction ? contentKey(widget.options.faction) : null;
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const pick = groups.data ? chosenFaction(groups.data, saved, tag, hour) : null;
  return { groups, pick, tag };
}

/** The faction's held state at one hour, as Factions' details read it. */
export function useFactionDetails(runtime: WidgetRuntime, track: BoundTrack | undefined, faction: string | null, hour: number, active: boolean) {
  return useProduct<FactionDetailReply>(runtime.client, track && faction ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'faction-details', params: { branchId: track.branch.id, hour: Math.floor(hour), faction } } : null, active);
}

/** Every recorded faction, by its latest name, for the Faction setting. */
export function useFactionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const groups = useCountryGroups(runtime, track, true);
  return useMemo(() => ({ faction: (groups.data?.factions ?? []).map(faction => ({ value: setToken(faction.id),
    label: factionNameAt(faction, faction.lastHour) })) }), [groups.data]);
}

/** The state words every faction widget uses when no faction is shown. */
export function noFactionMessage(reason: 'none' | 'unobserved' | 'unknown', live: boolean): string {
  return reason === 'unobserved' ? 'Faction rosters were not recorded at this time.'
    : reason === 'unknown' ? 'This country’s faction is not known at this time.'
      : live ? 'This country is in no faction yet.' : 'This country is in no faction at this date.';
}
