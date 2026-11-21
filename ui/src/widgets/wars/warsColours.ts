import { countryLineColours } from '../../dashboard/countryLineColours.ts';
import { runAt } from '../../charts/series.ts';
import { lookPlan } from '../../dashboard/countryLooks.ts';
import type { RecordedBranch } from '../../dashboard/countryLooks';
import { nameKey } from '../../dashboard/countryFacts.ts';
import type { Lineage } from '../../dashboard/identity';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryGroupReply, CountryStyle } from '../../generated/dashboard';
import { countryTagAt, observationAt } from './warsData.ts';
import type { NativeSide, PreparedWars } from './warsData';

export function warsFactionMembership(groups: CountryGroupReply | undefined, at: number, lineage: Lineage, identities?: CountryIdentityIndex): ReadonlyMap<string, string> | null {
  if (!groups || runAt(groups.unobserved.factions, at)) return null;
  const membership = new Map<string, string>();
  for (const faction of groups.factions) {
    if (at < faction.firstHour || at > faction.lastHour) continue;
    if (runAt(faction.incomplete, at)) return null;
    for (const run of faction.members) {
      if (at < run.firstHour || at > run.lastHour) continue;
      const country = identities ? identities.keyAt(run.tag, at) : lineage.countryAt(run.tag, at);
      if (country === null) return null;
      const earlier = membership.get(country);
      if (earlier !== undefined && earlier !== faction.id) return null;
      membership.set(country, faction.id);
    }
  }
  return membership;
}

export function warsSideColours(data: PreparedWars, groups: CountryGroupReply | undefined, at: number, colours: ReadonlyMap<string, string>): ReadonlyMap<string, ReadonlyMap<NativeSide, string>> {
  const result = new Map<string, ReadonlyMap<NativeSide, string>>();
  const membership = warsFactionMembership(groups, at, data.lineage, data.identities);
  if (!membership) return result;
  for (const view of data.wars) {
    const observation = observationAt(view.war, at);
    if (!observation) continue;
    const sides = new Map<NativeSide, string>();
    for (const side of ['viewpoint', 'opponent'] as const) {
      const countries = new Set((side === 'viewpoint' ? observation.viewpointSide : observation.opposingSide)
        .map(tag => data.identities ? data.identities.keyAt(tag, at) : data.lineage.countryAt(tag, at)));
      if (countries.has(null)) continue;
      const counts = new Map<string, number>();
      for (const country of countries) {
        if (country === null) continue;
        const faction = membership.get(country);
        if (faction !== undefined) counts.set(faction, (counts.get(faction) ?? 0) + 1);
      }
      for (const [faction, count] of counts) if (count > countries.size / 2) {
        const colour = colours.get(faction);
        if (colour !== undefined) sides.set(side, colour);
        break;
      }
    }
    if (sides.size) result.set(view.war.id, sides);
  }
  return result;
}

export function warsColourSelection(data: PreparedWars, at: number): ReadonlyMap<string, string | null> {
  return new Map(data.countries.map(country => {
    const tag = countryTagAt(country, at);
    return [country.key, tag !== null && (data.identities ? data.identities.keyAt(tag, at) : data.lineage.countryAt(tag, at)) === country.key ? tag : null] as const;
  }).sort(([first], [second]) => compareTags(first, second)));
}

export function warsLookStart(data: PreparedWars | null, branch: RecordedBranch | undefined, tag: string, hour: number): number | null {
  if (data?.identities) {
    const run = runAt(data.identityBounds.get(tag) ?? [], hour);
    return run && branch ? Math.max(branch.firstHour, run.firstHour) : null;
  }
  return branch ? Math.max(branch.firstHour, data?.lineage.holderStart(tag, hour) ?? branch.firstHour) : null;
}

export function warsColourPlan(data: PreparedWars | null, branch: RecordedBranch | undefined, at: number, fixed: boolean) {
  const selection = fixed && data ? warsColourSelection(data, at) : new Map<string, string | null>();
  const plan = lookPlan(branch, [...selection.values()].flatMap(tag => tag == null ? [] : [tag]), at,
    (tag, hour) => warsLookStart(data, branch, tag, hour));
  return { ...plan, selection };
}

export function warsFixedColourKey(plan: ReturnType<typeof warsColourPlan>, styles: ReadonlyMap<string, CountryStyle | null | Error>): string | null {
  if (!plan.wanted.length || plan.wanted.some(item => !styles.has(nameKey(item.hour, item.tag)))) return null;
  const hours = new Map(plan.samples.map(sample => [sample.tag, sample.hour]));
  const inputs = [...plan.selection].map(([key, tag]) => {
    const hour = tag == null ? undefined : hours.get(tag);
    const style = tag == null || hour === undefined ? null : styles.get(nameKey(hour, tag));
    const color = style != null && !(style instanceof Error) ? style.color : null;
    return [key, tag, color] as const;
  });
  return inputs.some(([, , color]) => color != null) ? JSON.stringify(inputs) : null;
}

export function warsCountryColours(tags: readonly string[], color: (tag: string) => CountryStyle['color'], surface: string, fixed: boolean): Map<string, string> {
  const styles = new Map<string, CountryStyle>();
  for (const tag of tags) {
    const recorded = color(tag);
    styles.set(tag, { tag, name: null, color: recorded });
  }
  const colours = countryLineColours([...styles.keys()].sort(compareTags), styles, surface, fixed);
  return new Map([...colours].filter(([tag]) => styles.get(tag)?.color != null));
}
