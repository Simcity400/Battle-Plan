import { countryLineColours } from '../../dashboard/countryLineColours.ts';
import { MAX_NAME_HOURS, nameKey } from '../../dashboard/countryFacts.ts';
import { lookHour } from '../../dashboard/countryLooks.ts';
import type { RecordedBranch } from '../../dashboard/countryLooks';
import type { CountryStyle } from '../../generated/dashboard';
import { factionPartWitness } from './factionParts.ts';
import type { FactionParts } from './factionParts';

/** Fixed correction reserves one place per native holder in the complete census,
 * independently of the chosen faction and member rank. Only style chunks expand
 * to this census; names and flags still read mounted member rows. */
export function factionColourPlan(model: FactionParts, branch: RecordedBranch, hour: number, fixed: boolean) {
  const selection = new Map<string, { tag: string; hour: number; stretch: number } | null>();
  for (const country of fixed ? model.census : model.parts) {
    const witness = factionPartWitness(country, hour);
    const sample = witness && lookHour(branch, witness.hour, witness.firstHour);
    selection.set(country.id, witness && sample ? { tag: witness.tag, ...sample } : null);
  }
  const wanted = [...new Map([...selection.values()].flatMap(sample => sample ? [[nameKey(sample.hour, sample.tag), sample] as const] : [])).values()];
  return { selection, wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(sample => sample.hour)).size) };
}

/** A missing recorded colour stays neutral. The shared correction waits for the
 * entire census to answer so chunk arrival order cannot change a holder's colour. */
export function factionColourKey(plan: ReturnType<typeof factionColourPlan>, styles: ReadonlyMap<string, CountryStyle | null | Error>): string | null {
  if (plan.wanted.some(sample => !styles.has(nameKey(sample.hour, sample.tag)))) return null;
  return JSON.stringify([...plan.selection].map(([id, sample]) => {
    const style = sample && styles.get(nameKey(sample.hour, sample.tag));
    return [id, style != null && !(style instanceof Error) ? style.color : null];
  }));
}

export function factionCountryColours(key: string, surface: string, fixed: boolean): Map<string, string> {
  const inputs = JSON.parse(key) as [string, CountryStyle['color']][];
  if (!inputs.some(([, color]) => color != null)) return new Map();
  const styles = new Map(inputs.map(([id, color]) => [id, { tag: id, name: null, color }]));
  const colours = countryLineColours([...styles.keys()], styles, surface, fixed);
  return new Map([...colours].filter(([id]) => styles.get(id)?.color != null));
}

export function heldFactionColours(colours: ReadonlyMap<string, string>, before: ReturnType<typeof factionColourPlan>['selection'],
  after: ReturnType<typeof factionColourPlan>['selection'], styles: ReadonlyMap<string, CountryStyle | null | Error>): Map<string, string> {
  return new Map([...colours].filter(([id]) => {
    const old = before.get(id); const next = after.get(id);
    const style = next && styles.get(nameKey(next.hour, next.tag));
    return old != null && next != null && old.tag === next.tag && old.stretch === next.stretch
      && (style === undefined || style != null && !(style instanceof Error) && style.color != null);
  }));
}
