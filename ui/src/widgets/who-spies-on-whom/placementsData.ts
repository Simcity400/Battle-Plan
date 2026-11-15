import { foldChord } from '../../charts/chord/fold.ts';
import type { FoldedChord } from '../../charts/chord/fold';
import type { ChordHit } from '../../charts/chord/layout';
import { chordTotals } from '../../charts/chord/totals.ts';
import { countryNameDetails } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { compareCountries, countryKey } from '../../dashboard/intelligence.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { Country, Placement } from '../../generated/dashboard';

export const placementKey = (placement: Placement) => `${placement.owner.id}:${placement.target.id}`;

/** The placements shown: every pair when `tags` is null, else those touching a chosen country; most operatives first. */
export function placementRows(placements: readonly Placement[], tags: ReadonlySet<string> | null): Placement[] {
  const shown = tags ? placements.filter(placement => [placement.owner.tag, placement.target.tag].some(tag => tag !== null && tags.has(tag))) : [...placements];
  return shown.sort((a, b) => b.operatives.length - a.operatives.length || compareCountries(a.owner, b.owner) || compareCountries(a.target, b.target));
}

export const operativeNames = (placement: Placement) => placement.operatives.map(operative => operative.name ?? 'Unnamed operative').join(', ');

/** What the chord's structure is made of: the countries it may draw and why nothing is known, never the counts. */
export function placementStructure(reply: { placements: readonly Placement[]; missing: string | null }): string {
  const countries = new Set<string>();
  for (const placement of reply.placements) for (const side of [placement.owner, placement.target]) countries.add(countryKey(side));
  return `${reply.missing ?? ''}|${[...countries].sort(compareTags).join(',')}`;
}

export const countryNodeId = (country: Country) => JSON.stringify(['country', countryKey(country)]);
export const OTHER_ID = JSON.stringify(['other']);

export type PlacementLook = CountryFlagLook & { name: string };

export interface PlacementChord extends FoldedChord {
  placements: Map<string, Placement>;
  countries: Map<string, Country>;
  totals: Map<string, { incoming: bigint; outgoing: bigint }>;
  look: (country: Country) => PlacementLook;
}

/** The chord of `placements`: a ribbon per pair, coloured by the country spying, the `most` countries with the most
 * operatives sent and received drawn, the rest folded into "Other countries". Counts are exact. */
export function placementChord(placements: readonly Placement[], most: number, look: (country: Country) => PlacementLook,
  color: (country: Country) => string, muted: string): PlacementChord {
  const totals = chordTotals(placements, n => countryNodeId(n.owner), n => countryNodeId(n.target), n => BigInt(n.operatives.length));
  const countries = new Map<string, Country>();
  for (const placement of placements) for (const side of [placement.owner, placement.target]) countries.set(countryNodeId(side), side);
  const sum = (id: string) => { const total = totals.get(id)!; return total.incoming + total.outgoing; };
  const ranked = [...countries.keys()].sort((a, b) => sum(a) === sum(b) ? compareCountries(countries.get(a)!, countries.get(b)!) : sum(a) > sum(b) ? -1 : 1);
  const keep = new Set(ranked.slice(0, Math.max(1, most)));
  const nodes = [...countries].sort((a, b) => compareCountries(a[1], b[1])).map(([id, country]) => {
    const shown = look(country);
    return { id, label: shown.name, flag: shown.flag, color: color(country), countryFlag: shown };
  });
  const flows = placements.map(n => ({ id: placementKey(n), source: countryNodeId(n.owner), target: countryNodeId(n.target), value: n.operatives.length }));
  const folded = foldChord({ nodes, flows }, keep, { id: OTHER_ID, label: 'Other countries', color: muted });
  return { ...folded, placements: new Map(placements.map(n => [placementKey(n), n])), countries, totals, look };
}

export function placementTooltip(hit: ChordHit, chord: PlacementChord): TooltipContent | null {
  const named = (country: Country) => chord.look(country);
  const title = (n: Placement) => `${named(n.owner).name} has ${n.operatives.length} ${n.operatives.length === 1 ? 'operative' : 'operatives'} in ${named(n.target).name}`;
  const names = (n: Placement) => n.operatives.map(o => ({ label: o.name ?? 'Unnamed operative' }));
  if (hit.kind === 'flow') {
    const members = chord.flows.get(hit.id)?.map(id => chord.placements.get(id)!);
    if (!members?.length) return null;
    if (members.length === 1) return { ...countryNameDetails(named(members[0].owner), named(members[0].target)), title: title(members[0]), rows: names(members[0]) };
    return { title: 'Operative placements', rows: [
      ...members.flatMap(n => [{ ...countryNameDetails(named(n.owner), named(n.target)), label: title(n), strong: true }, ...names(n)]),
      { label: 'Total operatives', value: members.reduce((sum, n) => sum + BigInt(n.operatives.length), 0n).toString(), strong: true },
    ] };
  }
  const memberIds = chord.nodes.get(hit.id);
  if (!memberIds) return null;
  const countries = memberIds.map(id => chord.countries.get(id)!);
  let incoming = 0n, outgoing = 0n;
  for (const id of memberIds) { const total = chord.totals.get(id)!; outgoing += total.outgoing; incoming += total.incoming; }
  return { ...countryNameDetails(...countries.map(named)), title: hit.id === OTHER_ID ? 'Other countries' : named(countries[0]).name,
    rows: [{ label: 'Operatives sent', value: outgoing.toString() }, { label: 'Operatives received', value: incoming.toString() },
      ...(hit.id === OTHER_ID ? countries.map(country => ({ label: named(country).name, countryFlag: named(country), tone: 'muted' as const })) : [])] };
}
