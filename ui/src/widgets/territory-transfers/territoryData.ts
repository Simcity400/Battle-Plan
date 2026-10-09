import { sumFlowParts } from '../../charts/flow/sum.ts';
import type { SummedDiagram, SummedNode } from '../../charts/flow/sum';
import type { PeaceDeal, PeaceDealsReply, PeaceTransferGraph, PeaceTransferredState } from '../../generated/dashboard';
import { checkedPeaceVictoryPoints, peaceDealHour, peaceDealKnownAt } from '../../dashboard/peaceDealsData.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { lookHour } from '../../dashboard/countryLooks.ts';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import { nameKey } from '../../dashboard/countryNamesAt.ts';
import type { RecordedBranch } from '../../dashboard/countryLooks';

export type TransferMeasure = 'states' | 'victoryPoints';
/** Conference handles hold only within their campaign and branch. */
export const conferenceToken = (campaign: string, branch: string, id: string) => setToken(campaignToken(campaign, JSON.stringify([branch, id])));
export function conferenceChoiceId(token: string, campaign: string, branch: string): string | null {
  if (!token) return '';
  const decoded = contentKey(token);
  const prefix = `${campaign}:`;
  if (!decoded.startsWith(prefix)) return null;
  try {
    const identity = JSON.parse(decoded.slice(prefix.length));
    return Array.isArray(identity) && identity.length === 2 && identity[0] === branch && typeof identity[1] === 'string' ? identity[1] : null;
  } catch { return null; }
}
export interface TransferOccurrence extends PeaceTransferredState {
  from: string; to: string; derived: boolean | null;
  /** The supplying observation, independently of the outcome's actual date. */
  knownAt?: number;
  /** Live has not reached the VP read, or its read hour was not recorded. */
  victoryPointsPending?: boolean;
}
export interface TransferLink {
  id: string; from: string; to: string; source: string; target: string; states: readonly TransferOccurrence[];
  /** Preserve the product's fold order independently of the playback date index. */
  nativeStates?: readonly TransferOccurrence[];
  stateCount: number; victoryPoints: bigint | null;
}
export interface TransferData {
  reply: PeaceDealsReply;
  campaign: readonly TransferLink[];
  conferences: ReadonlyMap<string, readonly TransferLink[]>;
  hours: readonly number[];
  conferenceHours: readonly number[];
  victoryPointHours: readonly number[];
  countryOrder: ReadonlyMap<string, number>;
  colourReads: ReadonlyMap<string, readonly { firstHour: number; readHour: number; tag: string }[]>;
}
export interface TransferDiagram extends SummedDiagram<TransferLink> { missingVictoryPoints: number; nonpositiveVictoryPoints: number }

const occurrenceKey = (from: string, to: string, state: number, before: number, after: number) => JSON.stringify([from, to, state, before, after]);
const decoded = new WeakMap<PeaceDealsReply, TransferData>();

/** Join the graph to its item provenance once. A graph occurrence without a
 * matching item remains unknown rather than being labelled exact or derived. */
export function decodeTransfers(reply: PeaceDealsReply): TransferData {
  const held = decoded.get(reply);
  if (held) return held;
  const provenance = new Map<string, { derived: boolean; knownAt: number }>();
  for (const deal of reply.deals) for (const item of deal.items) if (item.change.kind === 'stateTransfer') {
    const change = item.change;
    const key = occurrenceKey(countryIdentityKey(change.fromCountry), countryIdentityKey(change.toCountry), change.state, item.dates.beforeHour, item.dates.afterHour);
    // Exact finalizer items supersede a matching derived observation.
    const previous = provenance.get(key);
    if (!previous || previous.derived && !item.derived) provenance.set(key, { derived: item.derived, knownAt: peaceDealKnownAt(deal) });
  }
  const graph = (value: PeaceTransferGraph): TransferLink[] => {
    const countries = new Map(value.nodes.map(node => [node.id, node.country]));
    return value.links.map(link => {
      const source = countryIdentityKey(countries.get(link.source)!);
      const target = countryIdentityKey(countries.get(link.target)!);
      const nativeStates = link.states.map(state => {
        const witness = provenance.get(occurrenceKey(source, target, state.state, state.dates.beforeHour, state.dates.afterHour));
        return { ...state, from: state.fromCountry.tag, to: state.toCountry.tag, derived: witness?.derived ?? null, knownAt: witness?.knownAt ?? state.dates.afterHour };
      });
      const states = [...nativeStates]
        .sort((a, b) => a.dates.afterHour - b.dates.afterHour || a.dates.beforeHour - b.dates.beforeHour || a.state - b.state);
      return { id: link.id, source, target, from: states.at(-1)!.from, to: states.at(-1)!.to, states, nativeStates, stateCount: link.stateCount,
        victoryPoints: link.victoryPoints == null ? null : BigInt(link.victoryPoints) };
    });
  };
  const campaign = graph(reply.transfers);
  const conferences = new Map(reply.deals.map(deal => [deal.id, graph(deal.transfers)]));
  const hours = [...new Set(campaign.flatMap(link => link.states.flatMap(state => [state.dates.afterHour, state.knownAt ?? state.dates.afterHour])))].sort((a, b) => a - b);
  const conferenceHours = [...new Set(reply.deals.flatMap(deal => [peaceDealHour(deal), peaceDealKnownAt(deal)]))].sort((a, b) => a - b);
  const victoryPointHours = [...new Set(campaign.flatMap(link => link.states.flatMap(state => state.victoryPointsReadHour == null ? [] : [state.victoryPointsReadHour])))].sort((a, b) => a - b);
  const countries = new Map<string, string>();
  const colourWitnesses = new Map<string, Map<string, { firstHour: number; readHour: number; tag: string }>>();
  for (const links of [campaign, ...conferences.values()]) for (const link of links) for (const state of link.states) for (const country of [state.fromCountry, state.toCountry]) {
    const key = countryIdentityKey(country);
    countries.set(key, country.tag);
    const reads = colourWitnesses.get(key) ?? new Map<string, { firstHour: number; readHour: number; tag: string }>();
    const firstHour = Math.max(state.dates.afterHour, state.knownAt ?? state.dates.afterHour);
    const readKey = nameKey(country.hour, country.tag);
    reads.set(readKey, { firstHour: Math.min(firstHour, reads.get(readKey)?.firstHour ?? Infinity), readHour: country.hour, tag: country.tag });
    colourWitnesses.set(key, reads);
  }
  const countryOrder = new Map([...countries].sort((a, b) => compareTags(a[1], b[1]) || a[0].localeCompare(b[0])).map(([key], index) => [key, index]));
  const colourReads = new Map([...colourWitnesses].map(([key, reads]) =>
    [key, [...reads.values()].sort((a, b) => a.firstHour - b.firstHour || a.readHour - b.readHour)] as const));
  const data = { reply, campaign, conferences, hours, conferenceHours, victoryPointHours, countryOrder, colourReads };
  decoded.set(reply, data);
  return data;
}

/** Full-history admission waits for publication; appearance still belongs to the event's actor. */
export function transferColourPlan(data: TransferData | null | undefined, branch: RecordedBranch | undefined, moment: number, fixed: boolean) {
  const samples = fixed && data && branch ? [...data.countryOrder.keys()].flatMap(key =>
    (data.colourReads.get(key) ?? []).flatMap(read => read.firstHour <= moment && lookHour(branch, read.readHour, undefined, true)
      ? [{ key, tag: read.tag, hour: read.readHour }] : [])) : [];
  const wanted = [...new Map(samples.map(sample => [nameKey(sample.hour, sample.tag), { tag: sample.tag, hour: sample.hour }])).values()];
  return { samples, wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(sample => sample.hour)).size) };
}

/** Last transfer the playhead has reached, so intervening playback ticks reuse
 * the diagram without another pass or another sort. */
export function transferCutoff(hours: readonly number[], hour: number): number {
  let low = 0;
  let high = hours.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (hours[middle] <= hour) low = middle + 1; else high = middle;
  }
  return low ? hours[low - 1] : -Infinity;
}

/** The current scope's occurrences, dated by their after read. VPs are summed
 * only when every contributing occurrence is known, including repeated states.
 * Live also waits for the separate read that supplied each VP value. */
export function transfersThrough(links: readonly TransferLink[], hour: number, victoryPointsHour = Infinity, from = -Infinity): TransferLink[] {
  const shown: TransferLink[] = [];
  for (const link of links) {
    let low = 0;
    let high = link.states.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (link.states[middle].dates.afterHour <= hour) low = middle + 1; else high = middle;
    }
    if (!low) continue;
    if (low === link.states.length && victoryPointsHour === Infinity && from === -Infinity && link.states.every(state => (state.knownAt ?? state.dates.afterHour) <= hour)) { shown.push(link); continue; }
    const read = (state: TransferOccurrence) => victoryPointsHour !== Infinity && state.victoryPoints != null && (state.victoryPointsReadHour == null || state.victoryPointsReadHour > victoryPointsHour)
      ? { ...state, victoryPoints: null, victoryPointsPending: true } : state;
    const reached = (state: TransferOccurrence) => state.dates.afterHour >= from && state.dates.afterHour <= hour && (state.knownAt ?? state.dates.afterHour) <= hour;
    const states = link.states.slice(0, low).filter(reached).map(read);
    if (!states.length) continue;
    const nativeStates = (link.nativeStates ?? link.states).filter(reached).map(read);
    const victoryPoints = checkedPeaceVictoryPoints(nativeStates);
    shown.push({ ...link, from: states.at(-1)!.from, to: states.at(-1)!.to, states, nativeStates, stateCount: states.length, victoryPoints });
  }
  return shown;
}

export function transferNodeCountries(links: readonly TransferLink[]) {
  const countries = new Map<string, TransferOccurrence['fromCountry']>();
  for (const link of links) for (const state of link.states) for (const country of [state.fromCountry, state.toCountry]) {
    const key = countryIdentityKey(country);
    if (country.hour > (countries.get(key)?.hour ?? -Infinity)) countries.set(key, country);
  }
  return countries;
}

export function transferLookPlan(countries: ReturnType<typeof transferNodeCountries>, branch: RecordedBranch | undefined) {
  const samples = branch ? [...countries].flatMap(([key, country]) => lookHour(branch, country.hour, undefined, true)
    ? [{ key, tag: country.tag, hour: country.hour }] : []) : [];
  const wanted = [...new Map(samples.map(sample => [nameKey(sample.hour, sample.tag), { tag: sample.tag, hour: sample.hour }])).values()];
  return { samples, wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(sample => sample.hour)).size) };
}

/** Shared flow-kit summing retains every contributing link and occurrence in
 * the "other" node. An unknown VP link never supplies a partial width. */
export function buildTransferDiagram(links: readonly TransferLink[], measure: TransferMeasure, capacity: number | null): TransferDiagram {
  let missingVictoryPoints = 0;
  let nonpositiveVictoryPoints = 0;
  const valued = [];
  for (const link of links) {
    if (measure === 'victoryPoints' && link.victoryPoints == null) { missingVictoryPoints++; continue; }
    const value = measure === 'states' ? link.stateCount : Number(link.victoryPoints);
    if (value <= 0) { if (measure === 'victoryPoints') nonpositiveVictoryPoints++; continue; }
    valued.push({ part: link, source: link.source, target: link.target, value });
  }
  // The canvas measures its own room before drawing countries. No provisional
  // top-N or unbounded first frame is needed while that measurement arrives.
  return { ...sumFlowParts(capacity == null ? [] : valued, Math.max(1, (capacity ?? 1) - 1)), missingVictoryPoints, nonpositiveVictoryPoints };
}

/** Describe every drawn ribbon, including every summed node, without a chosen
 * screen-reader limit. Tooltip content retains their individual occurrences. */
export function transferDiagramDescription(diagram: TransferDiagram, measure: TransferMeasure, nodeName: (node: SummedNode) => string, note?: string): string {
  const nodes = new Map(diagram.nodes.map(node => [node.id, node]));
  const listed = diagram.links.map(link => `${nodeName(nodes.get(link.source)!)} to ${nodeName(nodes.get(link.target)!)}, ${transferAmount(link.parts, measure)} ${measure === 'states' ? 'state occurrences' : 'victory points'}`);
  return `${listed.join('; ')}${note ? `. ${note}` : ''}`;
}

/** Conferences available to the scope picker, including active conferences whose
 * outcome is still unknown. The product already supplies newest-first order. */
export function visibleConferences(deals: readonly PeaceDeal[], hour: number, from = -Infinity): PeaceDeal[] {
  return deals.filter(deal => peaceDealHour(deal) >= from && peaceDealKnownAt(deal) <= hour);
}

/** A chosen conference stays chosen. A choice from another recording is explicit
 * missing data; automatic chooses the newest conference playback has reached. */
export function chosenConference(deals: readonly PeaceDeal[], id: string): PeaceDeal | null {
  return id ? deals.find(deal => deal.id === id) ?? null : deals[0] ?? null;
}

export function transferAmount(links: readonly TransferLink[], measure: TransferMeasure): string {
  if (measure === 'states') return links.reduce((sum, link) => sum + link.stateCount, 0).toLocaleString('en-US');
  let total = 0n;
  for (const link of links) {
    if (link.victoryPoints == null) return 'Not recorded';
    total += link.victoryPoints;
  }
  return total.toLocaleString('en-US');
}

export function transferProvenance(states: readonly TransferOccurrence[]): string {
  let exact = 0;
  let derived = 0;
  let unknown = 0;
  for (const state of states) if (state.derived === false) exact++; else if (state.derived) derived++; else unknown++;
  return [exact ? `${exact.toLocaleString('en-US')} exact ${exact === 1 ? 'transfer' : 'transfers'} captured by the finalizer` : '',
    derived ? `${derived.toLocaleString('en-US')} ${derived === 1 ? 'transfer' : 'transfers'} derived from what changed` : '',
    unknown ? `${unknown.toLocaleString('en-US')} ${unknown === 1 ? 'transfer has' : 'transfers have'} provenance not recorded` : ''].filter(Boolean).join('; ');
}

/** Caption counts are about links before "other" aggregation, never about a
 * partial sum presented as the entire campaign. */
export function victoryPointNote(diagram: Pick<TransferDiagram, 'missingVictoryPoints' | 'nonpositiveVictoryPoints'>): string | null {
  const missing = diagram.missingVictoryPoints;
  const nonpositive = diagram.nonpositiveVictoryPoints;
  return [missing ? `${missing.toLocaleString('en-US')} ${missing === 1 ? 'link lacks' : 'links lack'} recorded victory points at the moment shown and ${missing === 1 ? 'is' : 'are'} not drawn` : '',
    nonpositive ? `${nonpositive.toLocaleString('en-US')} ${nonpositive === 1 ? 'link has' : 'links have'} zero or negative recorded victory points and ${nonpositive === 1 ? 'has' : 'have'} no ribbon width` : ''].filter(Boolean).join('; ') || null;
}

/** A completed conference's gap warning belongs to its own observed interval,
 * rather than later unrelated campaign gaps. An active conference follows Live. */
export function transferGapWindow(conference: PeaceDeal | null, links: readonly TransferLink[], firstHour: number, moment: number): [number, number] {
  if (!conference) return [firstHour, moment];
  let first = conference.firstObservedHour;
  for (const link of links) for (const state of link.states) first = Math.min(first, state.dates.beforeHour);
  const completed = conference.captured != null || conference.endHour != null || conference.firstFinalizedHour != null;
  return [first, completed ? Math.min(moment, peaceDealHour(conference)) : moment];
}

/** Gaps in the inputs that identify and attribute peace transfers remain visible
 * beside a total, including when no transfer could be attributed. */
export function transferGapCount(reply: PeaceDealsReply, from: number, to: number): number {
  const spans = [...reply.unobserved.settlements, ...reply.unobserved.conferences, ...reply.unobserved.ownership, ...reply.unobserved.wars]
    .filter(span => span.firstHour <= to && span.lastHour >= from)
    .map(span => ({ first: Math.max(from, span.firstHour), last: Math.min(to, span.lastHour) }))
    .sort((a, b) => a.first - b.first || a.last - b.last);
  let count = 0;
  let last = -Infinity;
  for (const span of spans) { if (span.first > last + 1) count++; last = Math.max(last, span.last); }
  return count;
}
