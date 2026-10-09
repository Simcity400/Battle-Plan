import { countryLineColours } from '../../dashboard/countryLineColours.ts';
import { MAX_NAME_HOURS, nameKey } from '../../dashboard/countryFacts.ts';
import type { NameAt } from '../../dashboard/countryFacts';
import { lookHour } from '../../dashboard/countryLooks.ts';
import type { RecordedBranch } from '../../dashboard/countryLooks';
import type { CountryStyle } from '../../generated/dashboard';
import { identityCountryAt } from './tradeData.ts';
import type { TradeData } from './tradeData';

type ColourSample = NameAt & { stretch: number };

/** Full native census identities reserve stable slots, independent of diagram
 * ranks, filters and the same holder appearing on both sides. Styles only. */
export function tradeColourPlan(data: TradeData | null, branch: RecordedBranch | undefined, hour: number, fixed: boolean) {
  const selection = new Map<string, ColourSample | null>();
  if (data && fixed) for (const identity of [...data.countryFacts.keys()].sort()) {
    const fact = identityCountryAt(data, identity, hour);
    const sample = branch && fact ? lookHour(branch, fact.hour, fact.firstHour) : null;
    selection.set(identity, fact && sample ? { tag: fact.tag, ...sample } : null);
  }
  const unique = new Map([...selection.values()].flatMap(sample => sample ? [[nameKey(sample.hour, sample.tag), sample] as const] : []));
  const wanted = [...unique.values()];
  return { selection, wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(sample => sample.hour)).size) };
}

/** Correction runs only after all bounded style chunks answer, and only when
 * recorded colours change. Dates and arriving names never change palette slots. */
export function tradeFixedColourKey(plan: ReturnType<typeof tradeColourPlan>, styles: ReadonlyMap<string, CountryStyle | null | symbol>): string | null {
  if (plan.wanted.some(sample => !styles.has(nameKey(sample.hour, sample.tag)))) return null;
  return JSON.stringify([...plan.selection].map(([identity, sample]) => {
    const style = sample && styles.get(nameKey(sample.hour, sample.tag));
    return [identity, typeof style === 'object' && style != null ? style.color : null];
  }));
}

/** The shared appearance adjustment runs once over the complete identity set.
 * Missing native colours reserve a slot but display the widget's neutral colour. */
export function tradeCountryColours(key: string, surface: string, fixed: boolean): Map<string, string> {
  const inputs = JSON.parse(key) as [string, CountryStyle['color']][];
  if (!inputs.some(([, color]) => color != null)) return new Map();
  const styles = new Map(inputs.map(([identity, color]) => [identity, { tag: identity, name: null, color }]));
  const colours = countryLineColours([...styles.keys()], styles, surface, fixed);
  return new Map([...colours].filter(([identity]) => styles.get(identity)?.color != null));
}

/** Pending samples can retain a correction only within the same holder/alias and
 * recorded stretch; a reused tag cannot borrow a previous holder's colour. */
export function heldTradeColours(colours: ReadonlyMap<string, string>, previous: ReturnType<typeof tradeColourPlan>['selection'],
  next: ReturnType<typeof tradeColourPlan>['selection']): Map<string, string> {
  return new Map([...colours].filter(([identity]) => {
    const before = previous.get(identity); const after = next.get(identity);
    return before != null && after != null && before.tag === after.tag && before.stretch === after.stretch;
  }));
}
