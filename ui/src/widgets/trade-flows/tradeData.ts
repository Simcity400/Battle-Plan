import { contentTitle } from '../../dashboard/contentTitles.ts';
import { runAt } from '../../charts/series.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import type { Span } from '../../charts/series';
import type { TooltipRow } from '../../charts/tooltip';
import { formatAmount, rescale, rescaleSum } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { CountryGroupReply, HourSpan, PartialSpan, ResourceInfo, TradeFlowsReply } from '../../generated/dashboard';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { signed32, signed64, unsigned32 } from '../../dashboard/validation.ts';
import { groupRuns, groupUnobserved } from '../../dashboard/groups.ts';
import type { ResolvedCountries } from '../../dashboard/groups';
import type { CountrySelection } from '../../dashboard/model';

/** Who sells what to whom, from the `trade-flows` product: each flow is one seller's
 * deals for one resource with one buyer (see `crates/dashboard/src/products/
 * trade_flows.rs` for what the values are and the evidence). Values are exact
 * fixed-point integers; the diagram draws with numbers and every label and tooltip
 * reads the exact integers. */

export interface PreparedTradeReply extends TradeFlowsReply {
  summaries?: Record<string, ChartSummary>;
  prefixes?: Record<string, Record<string, string[]>>;
  coverageCounts?: Record<string, [number, number]>;
  bounds?: Record<string, Record<string, [number | null, number | null]>>;
  exactHours?: number[];
  basis?: number;
  scoped?: boolean;
  coverage?: { firstHour: number; lastHour: number; complete: boolean };
  /** Prepared runs ahead of the identity history naming their holders; the next prefix catches up. */
  holdersPending?: boolean;
  cumulativePending?: boolean;
}

const HOUR = signed32;
const COUNT = unsigned32;
const IDENTITY = /^\[(0|[1-9]\d*),(0|[1-9]\d*)\]$/;
const nativeIdentity = (value: unknown): value is string => typeof value === 'string' && IDENTITY.test(value)
  && (JSON.parse(value) as unknown[]).every(unsigned32);
const ordered = (spans: readonly { firstHour: number; lastHour: number }[]) =>
  spans.every((span, index) => span.firstHour <= span.lastHour && (index === 0 || spans[index - 1].lastHour < span.firstHour));

/** Checks the reply's shape: resources by ascending native index, countries sorted
 * in the data service's byte order (`compareTags`), each flow's parallel run arrays
 * of one length with runs in hour order that never overlap and exact raw amounts,
 * and reads and known hours in ascending order. */
export function validateTradeFlowsReply(data: unknown): TradeFlowsReply {
  const reply = data as TradeFlowsReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.resources) && reply.resources.every((item, index) => Number.isInteger(item.id) && typeof item.token === 'string' && item.token.length > 0
      && Number.isInteger(item.ordinal) && item.ordinal >= 1 && (index === 0 || reply.resources[index - 1].id < item.id)
      && (item.name === null || typeof item.name === 'string') && (item.icon === null || (typeof item.icon === 'string' && item.icon.startsWith('data:image/png;base64,'))))
    && Array.isArray(reply.countries) && reply.countries.every((tag, index) => typeof tag === 'string' && tag.length > 0 && (index === 0 || compareTags(reply.countries[index - 1], tag) < 0))
    && Array.isArray(reply.countryIdentities) && reply.countryIdentities.every((country, index) => typeof country.tag === 'string' && country.tag.length > 0
      && (index === 0 || compareTags(reply.countryIdentities[index - 1].tag, country.tag) < 0)
      && Array.isArray(country.runs) && country.runs.every(run => run != null && HOUR(run.firstHour) && HOUR(run.lastHour)
        && COUNT(run.slot) && COUNT(run.countryId)) && ordered(country.runs))
    && Array.isArray(reply.reads) && reply.reads.every((hour, index) => HOUR(hour) && (index === 0 || reply.reads[index - 1] < hour))
    && Array.isArray(reply.known) && reply.known.every(span => HOUR(span.firstHour) && HOUR(span.lastHour)) && ordered(reply.known)
    && Array.isArray(reply.partial) && reply.partial.every(span => HOUR(span.firstHour) && HOUR(span.lastHour) && COUNT(span.deals) && span.deals > 0) && ordered(reply.partial)
    && Array.isArray(reply.flows) && reply.flows.every(flow => {
      const runs = flow.firstHours?.length;
      return Number.isInteger(runs) && (runs > 0 || !!(reply as PreparedTradeReply).prefixes || !!(reply as PreparedTradeReply).summaries) && COUNT(flow.seller) && flow.seller < reply.countries.length && COUNT(flow.buyer) && flow.buyer < reply.countries.length
        && nativeIdentity(flow.sellerIdentity) && nativeIdentity(flow.buyerIdentity)
        && reply.resources.some(item => item.id === flow.resource)
        && [flow.sellerTags, flow.buyerTags, flow.lastHours, flow.deliveredRaw, flow.factories, flow.convoysAssigned, flow.convoysRequired, flow.deals].every(list => Array.isArray(list) && list.length === runs)
        && flow.firstHours.every((first, index) => HOUR(first) && HOUR(flow.lastHours[index]) && first <= flow.lastHours[index] && (index === 0 || flow.lastHours[index - 1] < first))
        && flow.deliveredRaw.every(raw => signed64(raw))
        && flow.sellerTags.every(tag => COUNT(tag) && tag < reply.countries.length) && flow.buyerTags.every(tag => COUNT(tag) && tag < reply.countries.length)
        && flow.factories.every(HOUR) && flow.convoysAssigned.every(HOUR) && flow.convoysRequired.every(HOUR) && flow.deals.every(COUNT);
    })
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The trade flows reply is malformed.');
  // Indexing also refuses contradictory holder aliases. Every numeric run must
  // name the same exact native holder in the shared census at its supplying read.
  const trade = decodeTrade(reply);
  const outrun = trade.flows.some(flow => flow.runs.some(run => (['seller', 'buyer'] as const).some(side => {
    const fact = runAt(flow[side === 'seller' ? 'sellerFacts' : 'buyerFacts'], run.firstHour);
    return !fact || fact.tag !== run[side === 'seller' ? 'sellerTag' : 'buyerTag'] || fact.lastHour < run.lastHour;
  })));
  if (!outrun) return reply;
  // A prefix can hold runs ahead of the identity history that names their holders.
  if ((reply as PreparedTradeReply).coverage?.complete === false) return { ...reply, holdersPending: true } as PreparedTradeReply;
  throw new Error('The trade flow names a holder outside its recorded identity.');
}

/** One stretch of a flow's recorded terms. */
export interface FlowRun {
  firstHour: number; lastHour: number; sellerTag: string; buyerTag: string;
  /** The daily amount, exact, and as a number for drawing. */
  raw: bigint; amount: number;
  factories: number; convoysAssigned: number; convoysRequired: number; deals: number;
}

/** One seller's sales of one resource to one buyer: its runs, with running sums so
 * any hour's total is a binary search. */
export interface Flow {
  key: string;
  /** Initial aliases; use flowCountryAt for a dated presentation. */
  seller: string; buyer: string;
  sellerIdentity: string; buyerIdentity: string;
  sellerFacts: readonly TradeCountrySpan[]; buyerFacts: readonly TradeCountrySpan[];
  resource: ResourceInfo;
  runs: FlowRun[];
  /** Amount × hours and hours before each run, for totals. */
  amountBefore: Float64Array; hoursBefore: Float64Array;
  resourceDecimals?: number;
  prepared?: { prefixes: Record<string, string[]>; bounds?: Record<string, [number | null, number | null]>; summary?: ChartSummary; scoped: boolean; cumulativePending?: boolean };

}

function withSums(flow: Omit<Flow, 'runs' | 'amountBefore' | 'hoursBefore'>, runs: FlowRun[]): Flow {
  const amountBefore = new Float64Array(runs.length);
  const hoursBefore = new Float64Array(runs.length);
  let amount = 0;
  let hours = 0;
  runs.forEach((run, index) => {
    const prefix = flow.prepared?.prefixes[run.firstHour - 1];
    if (prefix) { amount = Number(prefix[0]) / 10 ** flow.resourceDecimals!; hours = Number(prefix[5]); }
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
  /** Each resource's place in the game's order, for its colour. */
  resourceOrder: ReadonlyMap<number, number>;
}

export interface TradeCountrySpan extends Span { identity: string; tag: string }
export interface TradeCountryFact { identity: string; tag: string; hour: number; firstHour: number; lastHour: number }

/** Exact holder key; tags and separators in mod content cannot change it. */
export const flowKey = (sellerIdentity: string, buyerIdentity: string, resource: number) => JSON.stringify([sellerIdentity, buyerIdentity, resource]);

function countryAt(facts: readonly TradeCountrySpan[], hour: number): TradeCountryFact | null {
  let low = 0, high = facts.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (facts[middle].firstHour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  const fact = facts[found];
  // When absent now, show the latest supplying census date, never a future alias.
  return fact ? { ...fact, hour: Math.min(hour, fact.lastHour) } : null;
}
export const identityCountryAt = (data: TradeData, identity: string, hour: number): TradeCountryFact | null => countryAt(data.countryFacts.get(identity) ?? [], hour);
export const flowCountryAt = (flow: Flow, side: 'seller' | 'buyer', hour: number): TradeCountryFact | null => countryAt(side === 'seller' ? flow.sellerFacts : flow.buyerFacts, hour);

const decoded = new WeakMap<TradeFlowsReply, TradeData>();

/** The reply's flows as runs, decoded once per reply. */
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
      sellerFacts: countryFacts.get(flow.sellerIdentity) ?? [], buyerFacts: countryFacts.get(flow.buyerIdentity) ?? [], resource: resources.get(flow.resource)!,
      ...((reply as PreparedTradeReply).prefixes || (reply as PreparedTradeReply).summaries ? { resourceDecimals: reply.decimals, prepared: { prefixes: (reply as PreparedTradeReply).prefixes?.[flowKey(flow.sellerIdentity, flow.buyerIdentity, flow.resource)] ?? {},
        bounds: (reply as PreparedTradeReply).bounds?.[flowKey(flow.sellerIdentity, flow.buyerIdentity, flow.resource)],
        summary: (reply as PreparedTradeReply).summaries?.[flowKey(flow.sellerIdentity, flow.buyerIdentity, flow.resource)], scoped: (reply as PreparedTradeReply).scoped === true,
        cumulativePending: (reply as PreparedTradeReply).cumulativePending } } : {}) }, runs);
  });
  data = { reply, flows, countryFacts, identitiesByTag, resourceOrder: new Map(reply.resources.map((item, index) => [item.id, index])) };
  decoded.set(reply, data);
  return data;
}

/** Which countries' trade is shown: all countries, or chosen countries (fixed, or
 * a group's members at each hour as `membership`), counting a flow when either end
 * is chosen or, with `between`, only when both are. A group counted at each hour
 * counts nothing in the hours its membership was not recorded (`unobserved`). */
export type TradeScope = { kind: 'all' } | { kind: 'countries'; tags: readonly string[]; membership: ReadonlyMap<string, readonly Span[]> | null;
  unobserved?: readonly Span[]; countryIds?: ReadonlySet<string>; identityNote?: string; between: boolean };

/** Spans with `holes` cut out, in hour order. */
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

/** Where a set of spans and another overlap, or together cover. */
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

/** The flows a scope and a resource choice show (every resource when none is
 * chosen), cut to the hours their countries count, so a country counted at each
 * hour trades in the diagram only while it is a member. */
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
    if (!scope.membership || flow.prepared?.scoped) { selected.push(flow); continue; }
    const runs = clip(flow.runs, combine(membership.get(flow.sellerIdentity) ?? [], membership.get(flow.buyerIdentity) ?? [], scope.between));
    if (runs.length) selected.push(withSums(flow, runs));
  }
  return selected;
}

/** End/ever membership chooses native holders at the dates it counts, before
 * including those holders' whole histories. Hour membership still clips each read. */
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

/** Prepared prefixes are exact; sparse chart samples never establish a total. */
function preparedPrefix(flow: Flow, hour: number): bigint[] | null {
  if (!flow.prepared) return null;
  const exact = flow.prepared.prefixes[hour];
  if (exact) return exact.map(BigInt);
  const run = runAt(flow.runs, hour), before = run && flow.prepared.prefixes[run.firstHour - 1];
  if (!run || !before) return null;
  const duration = BigInt(Math.min(hour, run.lastHour) - run.firstHour + 1);
  return before.map((raw, index) => BigInt(raw) + duration * [run.raw, BigInt(run.factories), BigInt(run.convoysAssigned), BigInt(run.convoysRequired), BigInt(run.deals), 1n][index]);
}

/** Amount × hours and hours of a flow up to and including `hour`. */
export function flowPrefix(flow: Flow, hour: number): [number, number] {
  if (flow.prepared?.cumulativePending) return [NaN, NaN];
  const prepared = preparedPrefix(flow, hour);
  if (prepared) return [Number(prepared[0]) / 10 ** flow.resourceDecimals!, Number(prepared[5])];
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

/** A flow's size in the diagram at `hour`: its daily amount then (current), what it
 * delivered from `from` to then (total), or its average daily amount over the hours
 * it traded in that time (average). Zero when it did not trade. For drawing only:
 * `exactFlow` gives the numbers shown. `basis` may reuse flowPrefix(flow, from - 1). */
export function flowValue(flow: Flow, measure: Measure, from: number, hour: number, basis?: readonly [number, number]): number {
  if (measure === 'current') return runAt(flow.runs, hour)?.amount ?? 0;
  const preparedEnd = preparedPrefix(flow, hour), preparedStart = preparedPrefix(flow, from - 1);
  if (preparedEnd && preparedStart) {
    const amount = Number(preparedEnd[0] - preparedStart[0]) / 10 ** flow.resourceDecimals!;
    const hours = Number(preparedEnd[5] - preparedStart[5]);
    return hours <= 0 ? 0 : measure === 'total' ? amount / 24 : amount / hours;
  }
  const [amountEnd, hoursEnd] = flowPrefix(flow, hour);
  const [amountStart, hoursStart] = basis ?? flowPrefix(flow, from - 1);
  const amount = amountEnd - amountStart;
  const hours = hoursEnd - hoursStart;
  if (hours <= 0) return 0;
  // Amount × hours: a daily amount held 24 hours delivers it once.
  return measure === 'total' ? amount / 24 : amount / hours;
}

/** Exact sums of flows over [from, hour], by recorded hour: amount × hours and the
 * factories and convoys × hours, the hours traded, and the first and last hours. */
export interface FlowSums { amountHours: bigint; factoryHours: bigint; assignedHours: bigint; requiredHours: bigint; hours: number; first: number | null; last: number | null }

export function exactSums(flows: readonly Flow[], from: number, hour: number): FlowSums {
  if (flows.some(flow => flow.prepared?.cumulativePending)) throw new Error('Trade totals are still being prepared.');
  const sums: FlowSums = { amountHours: 0n, factoryHours: 0n, assignedHours: 0n, requiredHours: 0n, hours: 0, first: null, last: null };
  for (const flow of flows) {
    const end = preparedPrefix(flow, hour), before = preparedPrefix(flow, from - 1);
    if (end && before) {
      sums.amountHours += end[0] - before[0]; sums.factoryHours += end[1] - before[1];
      sums.assignedHours += end[2] - before[2]; sums.requiredHours += end[3] - before[3]; sums.hours += Number(end[5] - before[5]);
      const dates = flow.prepared?.bounds?.[hour];
      if (dates) {
        if (dates[0] !== null) sums.first = Math.min(sums.first ?? Infinity, dates[0]);
        if (dates[1] !== null) sums.last = Math.max(sums.last ?? -Infinity, dates[1]);
        continue;
      }
      const runs = flow.runs.filter(run => run.firstHour <= hour && run.lastHour >= from);
      if (runs.length) { sums.first = Math.min(sums.first ?? Infinity, Math.max(from, runs[0].firstHour)); sums.last = Math.max(sums.last ?? -Infinity, Math.min(hour, runs.at(-1)!.lastHour)); }
      continue;
    }
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

/** The terms of flows at one hour, summed: exact amount, factories, convoys, deals. */
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

/** Hours of [from, to] the deals are known, from the reply's known spans. */
export function knownHours(known: readonly HourSpan[], from: number, to: number): number {
  let hours = 0;
  for (const span of known) hours += Math.max(0, Math.min(to, span.lastHour) - Math.max(from, span.firstHour) + 1);
  return hours;
}

/** Deals left out at an hour (a read listed them but could not attribute them), from
 * the reply's partial spans; 0 when the hour's deals are all attributed. */
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

/** Hours of [from, to] known only in part. */
export function partialHours(partial: readonly PartialSpan[], from: number, to: number): number {
  return knownHours(partial, from, to);
}

/** What the caption and tooltips say of how much of the trade shown was recorded:
 * deals left out at the moment (current), or days not recorded and days known only
 * in part over [from, hour] (total, average); none when everything was. */
export function coverageNote(reply: Pick<TradeFlowsReply, 'known' | 'partial'>, measure: Measure, from: number, hour: number): string | undefined {
  if (measure === 'current') {
    const left = leftOutAt(reply.partial, hour);
    return left ? `${usNumber.format(left)} trades could not be identified.` : undefined;
  }
  const prepared = reply as PreparedTradeReply;
  const count = prepared.basis === from ? prepared.coverageCounts?.[hour] : undefined;
  const partial = count?.[1] ?? partialHours(reply.partial, from, hour);
  const missing = count ? Math.max(0, hour - from + 1 - count[0]) : null;
  return [missing === null ? unknownNote(reply.known, from, hour) : missing >= 1 ? `Trade data is missing for ${formatDays(missing)}.` : undefined,
    partial >= 1 ? `Some trades are missing for ${formatDays(partial)}.` : undefined]
    .filter(Boolean).join(' ') || undefined;
}

/** Why nothing shows, when that is because nothing was recorded: the deals at the
 * moment (current), or over the whole of [from, hour] (total, average); null when
 * something was recorded. */
export function unrecordedMessage(known: readonly HourSpan[], measure: Measure, from: number, hour: number): string | null {
  if (measure === 'current') return knownSpanAt(known, hour) < 0 ? 'Trade deals were not recorded at this time.' : null;
  return knownHours(known, from, hour) < 1 ? 'Trade deals were not recorded in this time.' : null;
}

/** What an empty diagram says when something was recorded: whether all of it was
 * (no trade), or only part (deals a read could not attribute, days not recorded, or
 * a group counted at each hour whose membership was not recorded), so an incomplete
 * record never reads as no trade. `groupUnobserved` is the group's unrecorded
 * hours, or null for countries not chosen by such a group. */
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

/** The known span holding an hour, by its place, or -1 when the hour is unknown. */
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

/** The node that sums the countries past a side's limit. */
export const OTHER = '\u0000other';

/** The kit's reason to show before a request with no countries can load. */
export function selectionMessage(resolved: { reason: string | null } | null, atMoment: { reason: string | null },
  measure: Measure, from: number, hour: number, groupUnobserved: readonly Span[] | null = null): string | null {
  if (!resolved?.reason) return null;
  // An empty roster now does not explain missing membership earlier in the measured time.
  const missing = measure !== 'current' && groupUnobserved ? knownHours(groupUnobserved, from, hour) : 0;
  if (missing >= 1) return `Group membership is missing for ${formatDays(missing)}.`;
  return atMoment.reason ?? resolved.reason;
}

/** A native holder or the rest of one side, with exact supplying label dates. */
export interface SideNode {
  id: string; side: 'source' | 'target'; identity: string | null; tag: string | null; factHour: number | null;
  members: string[]; memberValues: Map<string, number>; memberFacts: Map<string, TradeCountryFact>; value: number;
}
/** A ribbon of the diagram: the flows between two nodes for one resource. */
export interface SideLink { id: string; source: string; target: string; resource: ResourceInfo; flows: Flow[]; value: number }
export interface Diagram { nodes: SideNode[]; links: SideLink[]; total: number }

const nodeId = (side: 'source' | 'target', identity: string | null) => `${side === 'source' ? 's' : 't'}:${identity ?? OTHER}`;
const factOrder = (facts: ReadonlyMap<string, TradeCountryFact>, a: string, b: string) => compareTags(facts.get(a)?.tag ?? '', facts.get(b)?.tag ?? '') || compareTags(a, b);

/** Positive amounts draw geometry. Exact signed/zero rows retain their flow keys. */
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

/** Fixed native-holder order, ranked by whole-window totals and dated aliases. */
export function sideRanks(flows: readonly Flow[], from: number, to: number): { source: Map<string, number>; target: Map<string, number> } {
  const sellers = new Map<string, number>(), buyers = new Map<string, number>();
  const facts = new Map<string, TradeCountryFact>();
  for (const flow of flows) {
    const total = flow.prepared?.cumulativePending ? 0 : flowValue(flow, 'total', from, to);
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

/** The value that fills the diagram's height for one scale all campaign, never below
 * what a side carries at any hour of [from, to]: the largest sum of daily amounts
 * at once (current), the total at the end, which only grows (total), or the sum of
 * each flow's highest running average (average). */
export function campaignScale(flows: readonly Flow[], measure: Measure, from: number, to: number): number {
  if (measure === 'total') return flows.reduce((sum, flow) => sum + flowValue(flow, 'total', from, to), 0);
  if (flows.some(flow => flow.prepared?.summary && (flow.prepared.summary.runs === undefined || !!flow.prepared.summary.approximate?.length))) {
    if (measure === 'average') return flows.reduce((sum, flow) => sum + Math.max(0, ...(flow.prepared?.summary?.buckets.flatMap(bucket => bucket ? [bucket.maxValue] : []) ?? [])), 0);
    const columns = Math.max(0, ...flows.map(flow => flow.prepared?.summary?.buckets.length ?? 0));
    let largest = 0;
    for (let column = 0; column < columns; column++) largest = Math.max(largest, flows.reduce((sum, flow) => sum + Math.max(0, flow.prepared?.summary?.buckets[column]?.maxValue ?? 0), 0));
    return largest;
  }
  if (measure === 'current') {
    // The largest sum of daily amounts at any hour: a sweep over where runs start and end.
    const events: [number, number][] = [];
    for (const flow of flows) for (const run of flow.prepared?.summary?.runs?.map(run => ({ ...run, amount: run.value })) ?? flow.runs) {
      if (run.lastHour < from || run.firstHour > to) continue;
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
  // A flow's running average moves only toward the amount of the run it is in, so it
  // peaks at the end of a run (cut to the time shown): its exact peak. The sum of the
  // flows' peaks is never below what the diagram reaches at any hour.
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
      peak = Math.max(peak, amount / hours);
    }
    most += peak;
  }
  return most;
}

/** A campaign scale kept for one scope (`key`). */
export interface HeldScale { key: string; scale?: number }

/** The campaign scale to draw: the one `read` now, else the last one read for the same
 * scope while another extent's is read; none until that scope's first is read. */
export function heldScale(held: HeldScale | undefined, key: string, read: number | undefined): HeldScale {
  if (read !== undefined) return held?.key === key && held.scale === read ? held : { key, scale: read };
  return held?.key === key ? held : { key };
}

/** Each resource's colour by its place in the campaign's full list (`order`, the
 * game's order), whatever is shown: the first as many as the palette has take its
 * colours in turn, and the rest share `other`, as a ninth category folds into
 * "Other". A filter never repaints a resource. */
export function resourceColours(resources: readonly ResourceInfo[], order: ReadonlyMap<number, number>, palette: readonly string[], other: string): Map<number, string> {
  return new Map(resources.map(item => {
    const place = order.get(item.id) ?? palette.length;
    return [item.id, place < palette.length ? palette[place] : other];
  }));
}

/** A resource's readable name, with repeated declarations numbered. */
export const resourceName = (item: Pick<ResourceInfo, 'name' | 'token' | 'ordinal'>) => `${contentTitle(item.name, item.token, 'Resource name unavailable')}${item.ordinal > 1 ? ` (${item.ordinal})` : ''}`;


/** Days as the game counts them, to a tenth where a day is not whole. */
export const formatDays = (hours: number) => {
  const days = Math.round(hours / 2.4) / 10;
  return `${usNumber.format(days)} ${days === 1 ? 'day' : 'days'}`;
};

/** The rows saying what flows carried: at an hour for the current amount, else over
 * [from, hour] (the total, or the average per day over the days traded), with the
 * civilian factories paid and convoys used then (on average, over time). */
export function measureRows(flows: readonly Flow[], measure: Measure, from: number, hour: number, decimals: number): TooltipRow[] {
  if (measure !== 'current' && flows.some(flow => flow.prepared?.cumulativePending)) return [{ label: 'Trade totals and averages are still being prepared.', tone: 'muted' }];
  if (measure === 'current') {
    const terms = termsAt(flows, hour);
    return [
      { label: 'Per day', value: formatAmount(terms.raw, decimals), strong: true },
      { label: 'Civilian factories paid', value: usNumber.format(terms.factories) },
      { label: 'Convoys used', value: terms.convoysRequired > 0 ? `${usNumber.format(terms.convoysAssigned)} of ${usNumber.format(terms.convoysRequired)}` : 'None needed' },
      ...(terms.deals > 1 ? [{ label: 'Deals', value: usNumber.format(terms.deals) }] : []),
    ];
  }
  // Each flow's averages over its own days traded, summed, as the ribbon is sized:
  // for several flows, what they carried side by side while trading.
  let amountHours = 0n;
  let perDay = 0n;
  const factories: [bigint, number][] = [];
  const assigned: [bigint, number][] = [];
  const required: [bigint, number][] = [];
  let traded = 0;
  let hours = 0;
  // Whether any factory was paid, convoy used or convoy needed, from the exact sums,
  // before anything is rounded for display.
  const any = { factories: false, assigned: false, required: false };
  for (const flow of flows) {
    const own = exactSums([flow], from, hour);
    if (!own.hours) continue;
    const length = BigInt(own.hours);
    traded++;
    hours = own.hours;
    amountHours += own.amountHours;
    perDay += rescale(own.amountHours, length, 0);
    // Keep fractions exact until their sum is rounded for display.
    factories.push([own.factoryHours, own.hours]);
    assigned.push([own.assignedHours, own.hours]);
    required.push([own.requiredHours, own.hours]);
    any.factories ||= own.factoryHours > 0n;
    any.assigned ||= own.assignedHours > 0n;
    any.required ||= own.requiredHours > 0n;
  }
  if (!traded) return [{ label: 'No trade recorded in this time', tone: 'muted' }];
  // An average above zero that rounds to nothing at a tenth says so.
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

/** Flows' exact amount by `measure`, as a fixed-point integer with the reply's
 * decimals: their daily amount at `hour`, what they delivered over [from, hour], or
 * the sum of each one's average per day over the hours it traded then. */
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

/** What a measure covers, for the tooltip's subtitle. */
export function measureNote(measure: Measure): string {
  return measure === 'current' ? 'Per day' : measure === 'total' ? 'Total delivered' : 'Average per trading day';
}

/** A note on unrecorded days inside [from, hour] for the totals and averages, which
 * cover recorded days only; none when every hour was recorded. */
export function unknownNote(known: readonly HourSpan[], from: number, hour: number): string | undefined {
  const missing = Math.max(0, hour - from + 1 - knownHours(known, from, hour));
  return missing >= 1 ? `Trade data is missing for ${formatDays(missing)}.` : undefined;
}

/** Exact point requests add dated facts while the accepted viewport keeps drawing. */
export function mergePreparedTrade(a: PreparedTradeReply, b: PreparedTradeReply): PreparedTradeReply {
  if (a.branchId !== b.branchId) throw new Error('Trade pages belong to different branches.');
  const countries = [...new Set([...a.countries, ...b.countries])].sort(compareTags), index = new Map(countries.map((tag, index) => [tag, index]));
  const reindex = (reply: PreparedTradeReply) => reply.flows.map(flow => ({ ...flow, seller: index.get(reply.countries[flow.seller])!, buyer: index.get(reply.countries[flow.buyer])!,
    sellerTags: flow.sellerTags.map(tag => index.get(reply.countries[tag])!), buyerTags: flow.buyerTags.map(tag => index.get(reply.countries[tag])!) }));
  const fields = ['firstHours', 'lastHours', 'sellerTags', 'buyerTags', 'deliveredRaw', 'factories', 'convoysAssigned', 'convoysRequired', 'deals'] as const;
  const extra = new Map(reindex(b).map(flow => [flowKey(flow.sellerIdentity, flow.buyerIdentity, flow.resource), flow]));
  const flows = reindex(a).map(flow => {
    const next = extra.get(flowKey(flow.sellerIdentity, flow.buyerIdentity, flow.resource));
    if (!next) return flow;
    extra.delete(flowKey(flow.sellerIdentity, flow.buyerIdentity, flow.resource));
    const rows = (source: typeof flow) => source.firstHours.map((firstHour, index) => ({ firstHour, lastHour: source.lastHours[index],
      values: fields.slice(2).map(field => source[field][index]) }));
    const joined = joinTradeSpans(rows(flow), rows(next), (x, y) => x.values.every((value, index) => value === y.values[index]));
    return { ...flow, firstHours: joined.map(row => row.firstHour), lastHours: joined.map(row => row.lastHour),
      ...Object.fromEntries(fields.slice(2).map((field, index) => [field, joined.map(row => row.values[index])])) };
  });
  flows.push(...extra.values());
  flows.sort((a, b) => a.seller - b.seller || a.buyer - b.buyer || a.resource - b.resource || a.sellerIdentity.localeCompare(b.sellerIdentity) || a.buyerIdentity.localeCompare(b.buyerIdentity));
  // Reuse the same authoritative point overlay for native slots, then regroup aliases.
  const census = (reply: PreparedTradeReply) => {
    const slots = new Map<number, { tag: string; slot: number; countryId: number; firstHour: number; lastHour: number }[]>();
    for (const country of reply.countryIdentities) for (const run of country.runs) {
      const rows = slots.get(run.slot) ?? []; rows.push({ ...run, tag: country.tag }); slots.set(run.slot, rows);
    }
    for (const rows of slots.values()) rows.sort((a, b) => a.firstHour - b.firstHour);
    return slots;
  };
  const first = census(a), second = census(b), aliases = new Map<string, TradeFlowsReply['countryIdentities'][number]['runs']>();
  for (const slot of new Set([...first.keys(), ...second.keys()])) {
    for (const row of joinTradeSpans(first.get(slot) ?? [], second.get(slot) ?? [], (a, b) => a.countryId === b.countryId && a.tag === b.tag)) {
      const runs = aliases.get(row.tag) ?? []; const { tag: _tag, ...run } = row; runs.push(run); aliases.set(row.tag, runs);
    }
  }
  const pages = <T>(first: Record<string, Record<string, T>> | undefined, second: Record<string, Record<string, T>> | undefined) =>
    Object.fromEntries([...new Set([...Object.keys(first ?? {}), ...Object.keys(second ?? {})])].map(key => [key, { ...first?.[key], ...second?.[key] }]));
  return { ...a, ...(a.holdersPending || b.holdersPending ? { holdersPending: true } : {}), flows, countries, resources: [...new Map([...a.resources, ...b.resources].map(resource => [resource.id, resource])).values()].sort((a, b) => a.id - b.id),
    countryIdentities: [...aliases].sort((a, b) => compareTags(a[0], b[0])).map(([tag, runs]) => ({ tag, runs: runs.sort((a, b) => a.firstHour - b.firstHour) })),
    reads: [...new Set([...a.reads, ...b.reads])].sort((a, b) => a - b),
    known: joinTradeSpans(a.known, b.known, () => true), partial: joinTradeSpans(a.partial, b.partial, (x, y) => x.deals === y.deals),
    prefixes: pages(a.prefixes, b.prefixes), bounds: pages(a.bounds, b.bounds), coverageCounts: { ...a.coverageCounts, ...b.coverageCounts },
    exactHours: [...new Set([...(a.exactHours ?? []), ...(b.exactHours ?? [])])] };
}

/** Point pages may overlap a fine viewport's complete runs. Keep one ordered
 * interval per fact, with gaps and the point's authoritative values preserved. */
function joinTradeSpans<T extends HourSpan>(first: readonly T[], second: readonly T[], same: (a: T, b: T) => boolean): T[] {
  const boundaries = [...new Set([...first, ...second].flatMap(row => [row.firstHour, row.lastHour + 1]))].sort((a, b) => a - b);
  const result: T[] = [];
  let a = 0, b = 0;
  for (let index = 0; index + 1 < boundaries.length; index++) {
    const hour = boundaries[index];
    while (a < first.length && first[a].lastHour < hour) a++;
    while (b < second.length && second[b].lastHour < hour) b++;
    const row = second[b]?.firstHour <= hour ? second[b] : first[a]?.firstHour <= hour ? first[a] : undefined;
    if (!row) continue;
    const previous = result.at(-1), lastHour = boundaries[index + 1] - 1;
    if (previous && previous.lastHour + 1 === hour && same(previous, row)) previous.lastHour = lastHour;
    else result.push({ ...row, firstHour: hour, lastHour });
  }
  return result;
}
