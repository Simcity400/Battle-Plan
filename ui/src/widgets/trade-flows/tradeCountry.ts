import { compareTags } from '../../dashboard/tagOrder.ts';
import type { HourSpan } from '../../generated/dashboard';
import { exactSums, flowCountryAt, knownHours, knownSpanAt, measureAmount, termsAt } from './tradeData.ts';
import type { Flow, Measure, TradeCountryFact, TradeData } from './tradeData';

/** One country's trade, from its own side: what it bought (imports) and sold (exports). */
export type TradeDirection = 'imports' | 'exports';
export interface CountryTrade { imports: Flow[]; exports: Flow[] }

/** The tags to ask the product for to have a native holder's whole trade, or null when
 * the reply already has it: every country's trade, or chosen tags one of the holder's
 * own aliases is among (the product expands a tag to every holder that ever had it). */
export function countryTradeTags(trade: Pick<TradeData, 'countryFacts'>, requested: readonly string[] | null, holder: string): string[] | null {
  if (!requested) return null;
  const aliases = [...new Set((trade.countryFacts.get(holder) ?? []).map(fact => fact.tag))].sort(compareTags);
  if (!aliases.length || aliases.some(tag => requested.includes(tag))) return null;
  return aliases;
}

/** The holder's flows each way, for the chosen resources (all when none is chosen),
 * in the game's resource order and then by partner, so a resource's bands sit together. */
export function countryTrade(trade: Pick<TradeData, 'flows' | 'resourceOrder'>, holder: string, resources: ReadonlySet<number> | null): CountryTrade {
  const order = (flow: Flow) => trade.resourceOrder.get(flow.resource.id) ?? Number.MAX_SAFE_INTEGER;
  const sorted = (partner: (flow: Flow) => string) => (a: Flow, b: Flow) => order(a) - order(b) || compareTags(partner(a), partner(b));
  const wanted = trade.flows.filter(flow => !resources || resources.has(flow.resource.id));
  return {
    imports: wanted.filter(flow => flow.buyerIdentity === holder).sort(sorted(flow => flow.sellerIdentity)),
    exports: wanted.filter(flow => flow.sellerIdentity === holder).sort(sorted(flow => flow.buyerIdentity)),
  };
}

/** The partner's side of a flow, seen from the holder's direction. */
export const partnerSide = (direction: TradeDirection) => direction === 'imports' ? 'seller' as const : 'buyer' as const;
export const partnerOf = (flow: Flow, direction: TradeDirection) => direction === 'imports' ? flow.sellerIdentity : flow.buyerIdentity;

/** One row of a country's trade: a partner (its native holder) or a resource (its id). */
export interface CountryPart { id: string; flows: Flow[]; raw: bigint }

/** Whether nothing of the measure is known at `hour`: the deals then (current), or
 * every hour of [from, hour] (total, average), were not recorded. */
export const countryMeasureUnknown = (known: readonly HourSpan[], measure: Measure, from: number, hour: number) =>
  measure === 'current' ? knownSpanAt(known, hour) < 0 : knownHours(known, from, hour) === 0;

/** The flows grouped by partner or by resource, with their exact measured amount.
 * A group is listed while one of its deals runs at the hour (current) or ran in
 * [from, hour] (total, average), a recorded zero amount included; groups with no
 * deal then are left out. None when nothing is known then (`countryTotal` is null). */
export function countryParts(flows: readonly Flow[], direction: TradeDirection, by: 'partner' | 'resource', measure: Measure,
  from: number, hour: number, known: readonly HourSpan[]): CountryPart[] {
  if (measure !== 'current' && flows.some(flow => flow.prepared?.cumulativePending)) return [];
  if (countryMeasureUnknown(known, measure, from, hour)) return [];
  const groups = new Map<string, Flow[]>();
  for (const flow of flows) {
    const id = by === 'partner' ? partnerOf(flow, direction) : String(flow.resource.id);
    const group = groups.get(id) ?? [];
    group.push(flow); groups.set(id, group);
  }
  const parts: CountryPart[] = [];
  for (const [id, group] of groups) {
    const traded = measure === 'current' ? termsAt(group, hour).flows > 0 : exactSums(group, from, hour).hours > 0;
    if (traded) parts.push({ id, flows: group, raw: measureAmount(group, measure, from, hour) });
  }
  return parts;
}

/** A direction's exact total, or null when it is not known then. */
export function countryTotal(flows: readonly Flow[], measure: Measure, from: number, hour: number, known: readonly HourSpan[]): bigint | null {
  if (measure !== 'current' && flows.some(flow => flow.prepared?.cumulativePending)) return null;
  return countryMeasureUnknown(known, measure, from, hour) ? null : measureAmount(flows, measure, from, hour);
}

/** The partner's dated holder fact for a group of its flows at `hour`. */
export function partnerFact(part: Pick<CountryPart, 'flows'>, direction: TradeDirection, hour: number): TradeCountryFact | null {
  for (const flow of part.flows) {
    const fact = flowCountryAt(flow, partnerSide(direction), hour);
    if (fact) return fact;
  }
  return null;
}

/** Each run of consecutive bands with one key (a resource), as its first and last
 * band: flows sorted by resource stack each resource's deals together. */
export function bandGroups(keys: readonly number[]): [number, number][] {
  const groups: [number, number][] = [];
  keys.forEach((key, index) => {
    const last = groups.at(-1);
    if (last && keys[last[1]] === key) last[1] = index; else groups.push([index, index]);
  });
  return groups;
}

/** One band per group, from its first band's lower edge to its last band's upper
 * edge: the deals' bands are positive and stacked, so the group's area is exact. */
export function mergedBands<T extends { bands: { low: number[]; high: number[] }[] }>(piece: T, groups: readonly (readonly [number, number])[]): T {
  return { ...piece, bands: groups.map(([first, last]) => ({ low: piece.bands[first].low, high: piece.bands[last].high })) };
}

/** Exports draw above the axis and imports below it: the import stack's edges, mirrored. */
export function mirrored<T extends { bands: { low: number[]; high: number[] }[] }>(piece: T): T {
  return { ...piece, bands: piece.bands.map(band => ({ low: band.high.map(value => -value), high: band.low.map(value => -value) })) };
}

/** The value axis covering both stacks: imports' extent below zero, exports' above. */
export function divergingExtent(exports: readonly [number, number] | null, imports: readonly [number, number] | null): [number, number] | null {
  if (!exports && !imports) return null;
  return [-(imports?.[1] ?? 0), exports?.[1] ?? 0];
}
