import { contentTitle } from '../../dashboard/contentTitles.ts';
import { artworkUrlValue } from '../../dashboard/payload.ts';
import { runAt } from '../../charts/series.ts';
import { catalogueColours } from '../../charts/palette.ts';
import type { Span } from '../../charts/series';
import type { TooltipRow } from '../../charts/tooltip';
import { formatAmount, rescale, rescaleSum } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { CountryGroupReply, HourSpan, PartialSpan, ResourceInfo, TradeFlow, TradeFlowsReply } from '../../generated/dashboard';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { signed32, signed64, unsigned32 } from '../../dashboard/validation.ts';
import { groupRuns, groupUnobserved } from '../../dashboard/groups.ts';
import type { ResolvedCountries } from '../../dashboard/groups';
import type { CountrySelection } from '../../dashboard/model';


const HOUR = signed32;
const COUNT = unsigned32;
const IDENTITY = /^\[(0|[1-9]\d*),(0|[1-9]\d*)\]$/;
const nativeIdentity = (value: unknown): value is string => typeof value === 'string' && IDENTITY.test(value)
  && (JSON.parse(value) as unknown[]).every(unsigned32);
const ordered = (spans: readonly { firstHour: number; lastHour: number }[]) =>
  spans.every((span, index) => span.firstHour <= span.lastHour && (index === 0 || spans[index - 1].lastHour < span.firstHour));

export function validateTradeFlowsReply(data: unknown): TradeFlowsReply {
  const reply = data as TradeFlowsReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && HOUR(reply.firstHour) && HOUR(reply.lastHour)
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.resources) && reply.resources.every((item, index) => Number.isInteger(item.id) && typeof item.token === 'string' && item.token.length > 0
      && Number.isInteger(item.ordinal) && item.ordinal >= 1 && (index === 0 || reply.resources[index - 1].id < item.id)
      && (item.name === null || typeof item.name === 'string') && (item.icon === null || artworkUrlValue(item.icon)))
    && Array.isArray(reply.countries) && reply.countries.every((tag, index) => typeof tag === 'string' && tag.length > 0 && (index === 0 || compareTags(reply.countries[index - 1], tag) < 0))
    && Array.isArray(reply.countryIdentities) && reply.countryIdentities.every((country, index) => typeof country.tag === 'string' && country.tag.length > 0
      && (index === 0 || compareTags(reply.countryIdentities[index - 1].tag, country.tag) < 0)
      && Array.isArray(country.runs) && country.runs.every(run => run != null && HOUR(run.firstHour) && HOUR(run.lastHour)
        && COUNT(run.slot) && COUNT(run.countryId)) && ordered(country.runs))
    && Array.isArray(reply.known) && reply.known.every(span => HOUR(span.firstHour) && HOUR(span.lastHour)) && ordered(reply.known)
    && Array.isArray(reply.partial) && reply.partial.every(span => HOUR(span.firstHour) && HOUR(span.lastHour) && COUNT(span.deals) && span.deals > 0) && ordered(reply.partial)
    && Array.isArray(reply.flows) && reply.flows.every(flow => {
      const runs = flow.firstHours?.length;
      return Number.isInteger(runs) && runs > 0 && COUNT(flow.seller) && flow.seller < reply.countries.length && COUNT(flow.buyer) && flow.buyer < reply.countries.length
        && nativeIdentity(flow.sellerIdentity) && nativeIdentity(flow.buyerIdentity)
        && reply.resources.some(item => item.id === flow.resource)
        && [flow.sellerTags, flow.buyerTags, flow.lastHours, flow.deliveredRaw, flow.factories, flow.convoysAssigned, flow.convoysRequired, flow.deals].every(list => Array.isArray(list) && list.length === runs)
        && flow.firstHours.every((first, index) => HOUR(first) && HOUR(flow.lastHours[index]) && first <= flow.lastHours[index] && (index === 0 || flow.lastHours[index - 1] < first))
        && flow.deliveredRaw.every(raw => signed64(raw))
        && flow.sellerTags.every(tag => COUNT(tag) && tag < reply.countries.length) && flow.buyerTags.every(tag => COUNT(tag) && tag < reply.countries.length)
        && flow.factories.every(Number.isSafeInteger) && flow.convoysAssigned.every(Number.isSafeInteger) && flow.convoysRequired.every(Number.isSafeInteger) && flow.deals.every(Number.isSafeInteger);
    })
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The trade flows reply is malformed.');
  const trade = decodeTrade(reply);
  const outrun = trade.flows.some(flow => flow.runs.some(run => (['seller', 'buyer'] as const).some(side => {
    const fact = runAt(flow[side === 'seller' ? 'sellerFacts' : 'buyerFacts'], run.firstHour);
    return !fact || fact.tag !== run[side === 'seller' ? 'sellerTag' : 'buyerTag'] || fact.lastHour < run.lastHour;
  })));
  if (!outrun) return reply;
  throw new Error('The trade flow names a holder outside its recorded identity.');
}

export interface FlowRun {
  firstHour: number; lastHour: number; sellerTag: string; buyerTag: string;
  raw: bigint; amount: number;
  factories: number; convoysAssigned: number; convoysRequired: number; deals: number;
}

export interface Flow {
  key: string;
  seller: string; buyer: string;
  sellerIdentity: string; buyerIdentity: string;
  sellerFacts: readonly TradeCountrySpan[]; buyerFacts: readonly TradeCountrySpan[];
  resource: ResourceInfo;
  runs: FlowRun[];
  amountBefore: Float64Array; hoursBefore: Float64Array;
}

function withSums(flow: Omit<Flow, 'runs' | 'amountBefore' | 'hoursBefore'>, runs: FlowRun[]): Flow {
  const amountBefore = new Float64Array(runs.length);
  const hoursBefore = new Float64Array(runs.length);
  let amount = 0;
  let hours = 0;
  runs.forEach((run, index) => {
    amountBefore[index] = amount;
    hoursBefore[index] = hours;
    const length = run.lastHour - run.firstHour + 1;
    amount += run.amount * length;
    hours += length;
  });
  return { ...flow, runs, amountBefore, hoursBefore };
}

export interface TradeData {
  reply: TradeFlowsReply;
  flows: Flow[];
  countryFacts: ReadonlyMap<string, readonly TradeCountrySpan[]>;
  identitiesByTag: ReadonlyMap<string, readonly TradeCountrySpan[]>;
  resourceOrder: ReadonlyMap<number, number>;
}

export interface TradeCountrySpan extends Span { identity: string; tag: string }
export interface TradeCountryFact { identity: string; tag: string; hour: number; firstHour: number; lastHour: number }

export const flowKey = (sellerIdentity: string, buyerIdentity: string, resource: number) => JSON.stringify([sellerIdentity, buyerIdentity, resource]);

function countryAt(facts: readonly TradeCountrySpan[], hour: number): TradeCountryFact | null {
  let low = 0, high = facts.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (facts[middle].firstHour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  const fact = facts[found];
  return fact ? { ...fact, hour: Math.min(hour, fact.lastHour) } : null;
}
export const identityCountryAt = (data: TradeData, identity: string, hour: number): TradeCountryFact | null => countryAt(data.countryFacts.get(identity) ?? [], hour);
export const flowCountryAt = (flow: Flow, side: 'seller' | 'buyer', hour: number): TradeCountryFact | null => countryAt(side === 'seller' ? flow.sellerFacts : flow.buyerFacts, hour);

const decoded = new WeakMap<TradeFlowsReply, TradeData>();

export function decodeTrade(reply: TradeFlowsReply): TradeData {
  let data = decoded.get(reply);
  if (data) return data;
  const countryFacts = new Map<string, TradeCountrySpan[]>();
  const identitiesByTag = new Map<string, TradeCountrySpan[]>();
  for (const country of reply.countryIdentities) for (const run of country.runs) {
    const identity = countryIdentityKey(run);
    const fact = { identity, tag: country.tag, firstHour: run.firstHour, lastHour: run.lastHour };
    const facts = countryFacts.get(identity) ?? [];
    facts.push(fact); countryFacts.set(identity, facts);
    const aliases = identitiesByTag.get(country.tag) ?? [];
    aliases.push(fact); identitiesByTag.set(country.tag, aliases);
  }
  for (const facts of countryFacts.values()) {
    facts.sort((a, b) => a.firstHour - b.firstHour);
    if (!ordered(facts)) throw new Error('The trade holder has contradictory recorded aliases.');
  }
  const resources = new Map(reply.resources.map(item => [item.id, item]));
  const scale = 10 ** reply.decimals;
  const flows = reply.flows.map(flow => {
    const seller = reply.countries[flow.seller];
    const buyer = reply.countries[flow.buyer];
    const runs = flow.firstHours.map((firstHour, index): FlowRun => {
      const raw = BigInt(flow.deliveredRaw[index]);
      return { firstHour, lastHour: flow.lastHours[index], sellerTag: reply.countries[flow.sellerTags[index]], buyerTag: reply.countries[flow.buyerTags[index]], raw, amount: Number(raw) / scale, factories: flow.factories[index],
        convoysAssigned: flow.convoysAssigned[index], convoysRequired: flow.convoysRequired[index], deals: flow.deals[index] };
    });
    return withSums({ key: flowKey(flow.sellerIdentity, flow.buyerIdentity, flow.resource), seller, buyer,
      sellerIdentity: flow.sellerIdentity, buyerIdentity: flow.buyerIdentity,
      sellerFacts: countryFacts.get(flow.sellerIdentity) ?? [], buyerFacts: countryFacts.get(flow.buyerIdentity) ?? [], resource: resources.get(flow.resource)! }, runs);
  });
  data = { reply, flows, countryFacts, identitiesByTag, resourceOrder: new Map(reply.resources.map((item, index) => [item.id, index])) };
  decoded.set(reply, data);
  return data;
}

export type TradeScope = { kind: 'all' } | { kind: 'countries'; tags: readonly string[]; membership: ReadonlyMap<string, readonly Span[]> | null;
  unobserved?: readonly Span[]; countryIds?: ReadonlySet<string>; identityNote?: string; between: boolean };

function without(spans: readonly Span[], holes: readonly Span[]): Span[] {
  if (!holes.length) return [...spans];
  const sorted = [...holes].sort((a, b) => a.firstHour - b.firstHour);
  const kept: Span[] = [];
  for (const span of spans) {
    let start = span.firstHour;
    for (const hole of sorted) {
      if (hole.lastHour < start || hole.firstHour > span.lastHour) continue;
      if (hole.firstHour > start) kept.push({ firstHour: start, lastHour: hole.firstHour - 1 });
      start = Math.max(start, hole.lastHour + 1);
    }
    if (start <= span.lastHour) kept.push({ firstHour: start, lastHour: span.lastHour });
  }
  return kept;
}

function combine(first: readonly Span[], second: readonly Span[], both: boolean): Span[] {
  const edges: [number, number][] = [];
  for (const span of first) edges.push([span.firstHour, 1], [span.lastHour + 1, -1]);
  for (const span of second) edges.push([span.firstHour, 1], [span.lastHour + 1, -1]);
  edges.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const need = both ? 2 : 1;
  const spans: Span[] = [];
  let depth = 0;
  let start: number | null = null;
  for (const [hour, step] of edges) {
    depth += step;
    if (depth >= need && start == null) start = hour;
    else if (depth < need && start != null) { if (hour > start) spans.push({ firstHour: start, lastHour: hour - 1 }); start = null; }
  }
  return spans;
}

function clip(runs: readonly FlowRun[], spans: readonly Span[]): FlowRun[] {
  const clipped: FlowRun[] = [];
  let at = 0;
  for (const run of runs) {
    while (at < spans.length && spans[at].lastHour < run.firstHour) at++;
    for (let index = at; index < spans.length && spans[index].firstHour <= run.lastHour; index++) {
      const first = Math.max(run.firstHour, spans[index].firstHour);
      const last = Math.min(run.lastHour, spans[index].lastHour);
      if (first <= last) clipped.push({ ...run, firstHour: first, lastHour: last });
    }
  }
  return clipped;
}

export function selectFlows(data: TradeData, scope: TradeScope, resources: ReadonlySet<number> | null): Flow[] {
  const wantedResource = (flow: Flow) => !resources || resources.has(flow.resource.id);
  if (scope.kind === 'all') return data.flows.filter(wantedResource);
  const chosen = scope.countryIds ?? new Set(scope.tags.flatMap(tag => (data.identitiesByTag.get(tag) ?? []).map(fact => fact.identity)));
  const membership = new Map<string, Span[]>();
  if (scope.membership) {
    for (const tag of scope.tags) for (const fact of data.identitiesByTag.get(tag) ?? []) {
      const spans = combine([fact], scope.membership.get(tag) ?? [], true);
      membership.set(fact.identity, [...(membership.get(fact.identity) ?? []), ...spans]);
    }
    for (const [identity, spans] of membership) membership.set(identity, without(combine(spans, [], false), scope.unobserved ?? []));
  }
  const selected: Flow[] = [];
  for (const flow of data.flows) {
    if (!wantedResource(flow)) continue;
    const [seller, buyer] = [chosen.has(flow.sellerIdentity), chosen.has(flow.buyerIdentity)];
    if (scope.between ? !(seller && buyer) : !(seller || buyer)) continue;
    if (!scope.membership) { selected.push(flow); continue; }
    const runs = clip(flow.runs, combine(membership.get(flow.sellerIdentity) ?? [], membership.get(flow.buyerIdentity) ?? [], scope.between));
    if (runs.length) selected.push(withSums(flow, runs));
  }
  return selected;
}

export function tradeScope(data: TradeData, selection: CountrySelection, resolved: ResolvedCountries | null,
  groups: CountryGroupReply | undefined, lastHour: number, between: boolean): TradeScope {
  if (selection.kind === 'all') return { kind: 'all' };
  const tags = resolved?.tags ?? [];
  const base = { kind: 'countries' as const, tags, membership: resolved?.membership ?? null, unobserved: resolved?.unobserved, between };
  if (selection.kind !== 'group' || selection.membership === 'hour') return base;
  const countryIds = new Set<string>();
  let missing = false;
  if (selection.membership === 'end') {
    for (const tag of tags) {
      const fact = runAt(data.identitiesByTag.get(tag) ?? [], lastHour);
      if (fact) countryIds.add(fact.identity); else missing = true;
    }
  } else if (groups) {
    const wanted = new Set(tags);
    const holes = groupUnobserved(groups, selection.group);
    for (const member of groupRuns(groups, selection.group) ?? []) {
      if (!wanted.has(member.tag)) continue;
      const facts = data.identitiesByTag.get(member.tag) ?? [];
      for (const fact of facts) {
        if (without(combine([fact], [member], true), holes).length) countryIds.add(fact.identity);
      }
      if (without(without([member], holes), facts).length) missing = true;
    }
  }
  return { ...base, countryIds, ...(missing ? { identityNote: 'Some trade could not be identified.' } : {}) };
}

export type Measure = 'current' | 'total' | 'average';

export function flowPrefix(flow: Flow, hour: number): [number, number] {
  const runs = flow.runs;
  let low = 0;
  let high = runs.length - 1;
  let found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (runs[middle].firstHour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  if (found < 0) return [0, 0];
  const run = runs[found];
  const hours = Math.min(hour, run.lastHour) - run.firstHour + 1;
  return [flow.amountBefore[found] + run.amount * hours, flow.hoursBefore[found] + hours];
}

export function flowValue(flow: Flow, measure: Measure, from: number, hour: number, basis?: readonly [number, number]): number {
  if (measure === 'current') return runAt(flow.runs, hour)?.amount ?? 0;
  const [amountEnd, hoursEnd] = flowPrefix(flow, hour);
  const [amountStart, hoursStart] = basis ?? flowPrefix(flow, from - 1);
  const amount = amountEnd - amountStart;
  const hours = hoursEnd - hoursStart;
  if (hours <= 0) return 0;
  return measure === 'total' ? amount / 24 : amount / hours;
}

export interface FlowSums { amountHours: bigint; factoryHours: bigint; assignedHours: bigint; requiredHours: bigint; hours: number; first: number | null; last: number | null }

export function exactSums(flows: readonly Flow[], from: number, hour: number): FlowSums {
  const sums: FlowSums = { amountHours: 0n, factoryHours: 0n, assignedHours: 0n, requiredHours: 0n, hours: 0, first: null, last: null };
  for (const flow of flows) {
    for (const run of flow.runs) {
      const first = Math.max(from, run.firstHour);
      const last = Math.min(hour, run.lastHour);
      if (first > last) continue;
      const hours = BigInt(last - first + 1);
      sums.amountHours += run.raw * hours;
      sums.factoryHours += BigInt(run.factories) * hours;
      sums.assignedHours += BigInt(run.convoysAssigned) * hours;
      sums.requiredHours += BigInt(run.convoysRequired) * hours;
      sums.hours += last - first + 1;
      sums.first = sums.first == null ? first : Math.min(sums.first, first);
      sums.last = sums.last == null ? last : Math.max(sums.last, last);
    }
  }
  return sums;
}

export interface FlowTerms { raw: bigint; factories: number; convoysAssigned: number; convoysRequired: number; deals: number; flows: number }

export function termsAt(flows: readonly Flow[], hour: number): FlowTerms {
  const terms: FlowTerms = { raw: 0n, factories: 0, convoysAssigned: 0, convoysRequired: 0, deals: 0, flows: 0 };
  for (const flow of flows) {
    const run = runAt(flow.runs, hour);
    if (!run) continue;
    terms.raw += run.raw;
    terms.factories += run.factories;
    terms.convoysAssigned += run.convoysAssigned;
    terms.convoysRequired += run.convoysRequired;
    terms.deals += run.deals;
    terms.flows++;
  }
  return terms;
}

export function knownHours(known: readonly HourSpan[], from: number, to: number): number {
  let hours = 0;
  for (const span of known) hours += Math.max(0, Math.min(to, span.lastHour) - Math.max(from, span.firstHour) + 1);
  return hours;
}

export function leftOutAt(partial: readonly PartialSpan[], hour: number): number {
  let low = 0;
  let high = partial.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (partial[middle].lastHour < hour) low = middle + 1;
    else if (partial[middle].firstHour > hour) high = middle - 1;
    else return partial[middle].deals;
  }
  return 0;
}

export function partialHours(partial: readonly PartialSpan[], from: number, to: number): number {
  return knownHours(partial, from, to);
}

export function coverageNote(reply: Pick<TradeFlowsReply, 'known' | 'partial'>, measure: Measure, from: number, hour: number): string | undefined {
  if (measure === 'current') {
    const left = leftOutAt(reply.partial, hour);
    return left ? `${usNumber.format(left)} trades could not be identified.` : undefined;
  }
  const partial = partialHours(reply.partial, from, hour);
  return [unknownNote(reply.known, from, hour),
    partial >= 1 ? `Some trades are missing for ${formatDays(partial)}.` : undefined]
    .filter(Boolean).join(' ') || undefined;
}

export function unrecordedMessage(known: readonly HourSpan[], measure: Measure, from: number, hour: number): string | null {
  if (measure === 'current') return knownSpanAt(known, hour) < 0 ? 'Trade deals were not recorded at this time.' : null;
  return knownHours(known, from, hour) < 1 ? 'Trade deals were not recorded in this time.' : null;
}

export function emptyMessage(reply: Pick<TradeFlowsReply, 'known' | 'partial'>, measure: Measure, from: number, hour: number,
  groupUnobserved: readonly Span[] | null = null): string {
  if (measure === 'current') {
    return leftOutAt(reply.partial, hour) ? 'No trade the recording could attribute at this moment.' : 'No trade at this moment.';
  }
  const span = hour - from + 1;
  const unobserved = groupUnobserved ? knownHours(groupUnobserved, from, hour) : 0;
  if (groupUnobserved && unobserved >= span) return 'This group’s members were not recorded in this time.';
  if (partialHours(reply.partial, from, hour) >= 1) return 'No trade the recording could attribute in this time.';
  if (knownHours(reply.known, from, hour) < span || unobserved >= 1) return 'No trade in the recorded part of this time.';
  return 'No trade in this time.';
}

export function knownSpanAt(known: readonly HourSpan[], hour: number): number {
  let low = 0;
  let high = known.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (known[middle].lastHour < hour) low = middle + 1;
    else if (known[middle].firstHour > hour) high = middle - 1;
    else return middle;
  }
  return -1;
}

export const OTHER = '\u0000other';

export function selectionMessage(resolved: { reason: string | null } | null, atMoment: { reason: string | null },
  measure: Measure, from: number, hour: number, groupUnobserved: readonly Span[] | null = null): string | null {
  if (!resolved?.reason) return null;
  const missing = measure !== 'current' && groupUnobserved ? knownHours(groupUnobserved, from, hour) : 0;
  if (missing >= 1) return `Group membership is missing for ${formatDays(missing)}.`;
  return atMoment.reason ?? resolved.reason;
}

export interface SideNode {
  id: string; side: 'source' | 'target'; identity: string | null; tag: string | null; factHour: number | null;
  members: string[]; memberValues: Map<string, number>; memberFacts: Map<string, TradeCountryFact>; value: number;
}
export interface SideLink { id: string; source: string; target: string; resource: ResourceInfo; flows: Flow[]; value: number }
export interface Diagram { nodes: SideNode[]; links: SideLink[]; total: number }

const nodeId = (side: 'source' | 'target', identity: string | null) => `${side === 'source' ? 's' : 't'}:${identity ?? OTHER}`;
const factOrder = (facts: ReadonlyMap<string, TradeCountryFact>, a: string, b: string) => compareTags(facts.get(a)?.tag ?? '', facts.get(b)?.tag ?? '') || compareTags(a, b);

export function buildDiagram(flows: readonly Flow[], measure: Measure, from: number, hour: number, perSide: number): Diagram {
  const valued = flows.map(flow => ({ flow, value: flowValue(flow, measure, from, hour) })).filter(item => item.value > 0);
  const facts = new Map<string, TradeCountryFact>();
  for (const { flow } of valued) for (const side of ['seller', 'buyer'] as const) {
    const fact = flowCountryAt(flow, side, hour);
    if (fact) facts.set(fact.identity, fact);
  }
  const totals = (pick: (flow: Flow) => string) => {
    const sums = new Map<string, number>();
    for (const { flow, value } of valued) sums.set(pick(flow), (sums.get(pick(flow)) ?? 0) + value);
    return sums;
  };
  const kept = (sums: Map<string, number>) => {
    const ranked = [...sums].sort((a, b) => b[1] - a[1] || factOrder(facts, a[0], b[0]));
    const limit = ranked.length <= perSide + 1 ? ranked.length : perSide;
    return new Set(ranked.slice(0, limit).map(([identity]) => identity));
  };
  const keptSellers = kept(totals(flow => flow.sellerIdentity));
  const keptBuyers = kept(totals(flow => flow.buyerIdentity));
  const nodes = new Map<string, SideNode>();
  const links = new Map<string, SideLink>();
  const node = (side: 'source' | 'target', identity: string, value: number, keep: Set<string>) => {
    const own = keep.has(identity) ? identity : null;
    const fact = facts.get(identity);
    const id = nodeId(side, own);
    const entry: SideNode = nodes.get(id) ?? { id, side, identity: own, tag: own ? fact?.tag ?? null : null, factHour: own ? fact?.hour ?? null : null,
      members: [], memberValues: new Map(), memberFacts: new Map(), value: 0 };
    entry.memberValues.set(identity, (entry.memberValues.get(identity) ?? 0) + value);
    if (fact) entry.memberFacts.set(identity, fact);
    entry.value += value;
    nodes.set(id, entry);
    return id;
  };
  let total = 0;
  for (const { flow, value } of valued) {
    total += value;
    const source = node('source', flow.sellerIdentity, value, keptSellers);
    const target = node('target', flow.buyerIdentity, value, keptBuyers);
    const id = JSON.stringify([source, target, flow.resource.id]);
    const link = links.get(id) ?? { id, source, target, resource: flow.resource, flows: [], value: 0 };
    link.flows.push(flow); link.value += value; links.set(id, link);
  }
  for (const entry of nodes.values()) entry.members = [...entry.memberValues].sort((a, b) => b[1] - a[1] || factOrder(facts, a[0], b[0])).map(([identity]) => identity);
  return { nodes: [...nodes.values()], links: [...links.values()], total };
}

export function sideRanks(flows: readonly Flow[], from: number, to: number): { source: Map<string, number>; target: Map<string, number> } {
  const sellers = new Map<string, number>(), buyers = new Map<string, number>();
  const facts = new Map<string, TradeCountryFact>();
  for (const flow of flows) {
    const total = flowValue(flow, 'total', from, to);
    sellers.set(flow.sellerIdentity, (sellers.get(flow.sellerIdentity) ?? 0) + total);
    buyers.set(flow.buyerIdentity, (buyers.get(flow.buyerIdentity) ?? 0) + total);
    for (const side of ['seller', 'buyer'] as const) {
      const fact = flowCountryAt(flow, side, to);
      if (fact) facts.set(fact.identity, fact);
    }
  }
  const rank = (sums: Map<string, number>) => new Map([...sums].sort((a, b) => b[1] - a[1] || factOrder(facts, a[0], b[0])).map(([identity], index) => [identity, index]));
  return { source: rank(sellers), target: rank(buyers) };
}

export function campaignScale(flows: readonly Flow[], measure: Measure, from: number, to: number): number {
  if (measure === 'current') {
    const events: [number, number][] = [];
    for (const flow of flows) for (const run of flow.runs) {
      if (run.lastHour < from || run.firstHour > to || !(run.amount > 0)) continue;
      events.push([Math.max(from, run.firstHour), run.amount], [Math.min(to, run.lastHour) + 1, -run.amount]);
    }
    events.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    let sum = 0;
    let most = 0;
    for (let index = 0; index < events.length; index++) {
      sum += events[index][1];
      if (index + 1 === events.length || events[index + 1][0] !== events[index][0]) most = Math.max(most, sum);
    }
    return most;
  }
  let most = 0;
  for (const flow of flows) {
    let amount = 0;
    let hours = 0;
    let peak = 0;
    for (const run of flow.runs) {
      const first = Math.max(from, run.firstHour);
      const last = Math.min(to, run.lastHour);
      if (first > last) continue;
      amount += run.amount * (last - first + 1);
      hours += last - first + 1;
      peak = Math.max(peak, measure === 'total' ? amount / 24 : amount / hours);
    }
    most += peak;
  }
  return most;
}

export interface HeldScale { key: string; scale?: number }

export function heldScale(held: HeldScale | undefined, key: string, read: number | undefined): HeldScale {
  if (read !== undefined) return held?.key === key && held.scale === read ? held : { key, scale: read };
  return held?.key === key ? held : { key };
}

export function resourceColours(resources: readonly ResourceInfo[], order: ReadonlyMap<number, number>, palette: readonly string[], surface?: string): Map<number, string> {
  const orderedIds = [...order.entries()].sort((a, b) => a[1] - b[1]).map(([id]) => id);
  for (const item of resources) if (!order.has(item.id)) orderedIds.push(item.id);
  const all = catalogueColours(orderedIds, surface, palette);
  return new Map(resources.map(item => [item.id, all.get(item.id)!]));
}

export const resourceName = (item: Pick<ResourceInfo, 'name' | 'token' | 'ordinal'>) => `${contentTitle(item.name, item.token, 'Resource name unavailable')}${item.ordinal > 1 ? ` (${item.ordinal})` : ''}`;


export const formatDays = (hours: number) => {
  const days = Math.round(hours / 2.4) / 10;
  return `${usNumber.format(days)} ${days === 1 ? 'day' : 'days'}`;
};

export function measureRows(flows: readonly Flow[], measure: Measure, from: number, hour: number, decimals: number): TooltipRow[] {
  if (measure === 'current') {
    const terms = termsAt(flows, hour);
    return [
      { label: 'Per day', value: formatAmount(terms.raw, decimals), strong: true },
      { label: 'Civilian factories paid', value: usNumber.format(terms.factories) },
      { label: 'Convoys used', value: terms.convoysRequired > 0 ? `${usNumber.format(terms.convoysAssigned)} of ${usNumber.format(terms.convoysRequired)}` : 'None needed' },
      ...(terms.deals > 1 ? [{ label: 'Deals', value: usNumber.format(terms.deals) }] : []),
    ];
  }
  let amountHours = 0n;
  let perDay = 0n;
  const factories: [bigint, number][] = [];
  const assigned: [bigint, number][] = [];
  const required: [bigint, number][] = [];
  let traded = 0;
  let hours = 0;
  const any = { factories: false, assigned: false, required: false };
  for (const flow of flows) {
    const own = exactSums([flow], from, hour);
    if (!own.hours) continue;
    const length = BigInt(own.hours);
    traded++;
    hours = own.hours;
    amountHours += own.amountHours;
    perDay += rescale(own.amountHours, length, 0);
    factories.push([own.factoryHours, own.hours]);
    assigned.push([own.assignedHours, own.hours]);
    required.push([own.requiredHours, own.hours]);
    any.factories ||= own.factoryHours > 0n;
    any.assigned ||= own.assignedHours > 0n;
    any.required ||= own.requiredHours > 0n;
  }
  if (!traded) return [{ label: 'No trade recorded in this time', tone: 'muted' }];
  const tenths = (values: readonly [bigint, number][], some = false) => {
    const rounded = rescaleSum(values, 1);
    return rounded === 0n && some ? '<0.1' : formatAmount(rounded, 1, 1);
  };
  return [
    measure === 'total'
      ? { label: 'Traded', value: formatAmount(rescale(amountHours, 24, 0), decimals), strong: true }
      : { label: 'Average per day', value: formatAmount(perDay, decimals), strong: true },
    traded > 1 ? { label: 'Trades', value: usNumber.format(traded) } : { label: 'Days traded', value: formatDays(hours) },
    { label: 'Civilian factories paid, on average', value: tenths(factories, any.factories) },
    { label: 'Convoys used, on average', value: any.required ? `${tenths(assigned, any.assigned)} of ${tenths(required, true)}` : 'None needed' },
  ];
}

export function measureAmount(flows: readonly Flow[], measure: Measure, from: number, hour: number): bigint {
  if (measure === 'current') return termsAt(flows, hour).raw;
  let amount = 0n;
  for (const flow of flows) {
    const own = exactSums([flow], from, hour);
    if (!own.hours) continue;
    amount += measure === 'total' ? own.amountHours : rescale(own.amountHours, BigInt(own.hours), 0);
  }
  return measure === 'total' ? rescale(amount, 24, 0) : amount;
}

export function measureNote(measure: Measure): string {
  return measure === 'current' ? 'Per day' : measure === 'total' ? 'Total delivered' : 'Average per trading day';
}

export function unknownNote(known: readonly HourSpan[], from: number, hour: number): string | undefined {
  const missing = Math.max(0, hour - from + 1 - knownHours(known, from, hour));
  return missing >= 1 ? `Trade data is missing for ${formatDays(missing)}.` : undefined;
}

export function compareFlows(a: TradeFlow, b: TradeFlow): number {
  return a.seller - b.seller || a.buyer - b.buyer || a.resource - b.resource
    || compareTags(a.sellerIdentity, b.sellerIdentity) || compareTags(a.buyerIdentity, b.buyerIdentity);
}

function joinSpans<T extends { firstHour: number; lastHour: number }>(spans: T[], next: readonly T[], same: (a: T, b: T) => boolean = () => true): void {
  for (const span of next) {
    const open = spans.at(-1);
    if (open && open.lastHour + 1 === span.firstHour && same(open, span)) spans[spans.length - 1] = { ...open, lastHour: span.lastHour };
    else spans.push({ ...span });
  }
}

export function mergeTrade(replies: readonly TradeFlowsReply[]): TradeFlowsReply | null {
  const held = [...replies].sort((a, b) => a.firstHour - b.firstHour);
  if (!held.length) return null;
  const countries = [...new Set(held.flatMap(reply => reply.countries))].sort(compareTags);
  const index = new Map(countries.map((tag, at) => [tag, at]));
  const flows = new Map<string, TradeFlow>();
  const identities = new Map<string, TradeFlowsReply['countryIdentities'][number]>();
  const known: HourSpan[] = [], partial: PartialSpan[] = [], warnings = new Set<string>();
  for (const reply of held) {
    const tag = (at: number) => index.get(reply.countries[at])!;
    for (const flow of reply.flows) {
      const key = JSON.stringify([flow.sellerIdentity, flow.buyerIdentity, flow.resource]);
      let into = flows.get(key);
      if (!into) flows.set(key, into = { ...flow, seller: tag(flow.seller), buyer: tag(flow.buyer), sellerTags: [], buyerTags: [], firstHours: [], lastHours: [],
        deliveredRaw: [], factories: [], convoysAssigned: [], convoysRequired: [], deals: [] });
      const joined = into;
      flow.firstHours.forEach((firstHour, run) => {
        const [seller, buyer] = [tag(flow.sellerTags[run]), tag(flow.buyerTags[run])];
        const open = joined.firstHours.length - 1;
        if (open >= 0 && joined.lastHours[open] + 1 === firstHour && joined.sellerTags[open] === seller && joined.buyerTags[open] === buyer
          && joined.deliveredRaw[open] === flow.deliveredRaw[run] && joined.factories[open] === flow.factories[run]
          && joined.convoysAssigned[open] === flow.convoysAssigned[run] && joined.convoysRequired[open] === flow.convoysRequired[run]
          && joined.deals[open] === flow.deals[run]) {
          joined.lastHours[open] = flow.lastHours[run];
          return;
        }
        joined.firstHours.push(firstHour); joined.lastHours.push(flow.lastHours[run]); joined.sellerTags.push(seller); joined.buyerTags.push(buyer);
        joined.deliveredRaw.push(flow.deliveredRaw[run]); joined.factories.push(flow.factories[run]); joined.convoysAssigned.push(flow.convoysAssigned[run]);
        joined.convoysRequired.push(flow.convoysRequired[run]); joined.deals.push(flow.deals[run]);
      });
    }
    for (const country of reply.countryIdentities) {
      let into = identities.get(country.tag);
      if (!into) identities.set(country.tag, into = { tag: country.tag, runs: [] });
      joinSpans(into.runs, country.runs, (a, b) => a.slot === b.slot && a.countryId === b.countryId);
    }
    joinSpans(known, reply.known);
    joinSpans(partial, reply.partial, (a, b) => a.deals === b.deals);
    for (const warning of reply.warnings) warnings.add(warning);
  }
  return { ...held[0], lastHour: Math.max(...held.map(reply => reply.lastHour)), countries,
    countryIdentities: [...identities.values()].sort((a, b) => compareTags(a.tag, b.tag)), known, partial, flows: [...flows.values()], warnings: [...warnings] };
}
