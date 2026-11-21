import type { HourSpan } from '../../generated/dashboard';
import { peaceDealHour, peaceDealIsBy, peaceDealKnownAt, peaceWarName } from '../../dashboard/peaceDealsData.ts';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import type { PeaceDeal, PeaceDealsReply, PeaceTransferCountry } from '../../generated/dashboard';

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

export interface TransferSide { key: string; tag: string; hour: number }
export interface TransferOccurrence {
  key: string; deal: string; state: number;
  from: TransferSide | null; to: TransferSide;
  beforeHour: number; afterHour: number;
  knownAt: number;
  victoryPoints: bigint | null; victoryPointsReadHour: number | null;
  victoryPointsPending?: boolean;
  released: boolean;
  derived: boolean | null;
}
export interface TransferData {
  reply: PeaceDealsReply;
  deals: ReadonlyMap<string, PeaceDeal>;
  all: readonly TransferOccurrence[];
  byDeal: ReadonlyMap<string, readonly TransferOccurrence[]>;
  hours: readonly number[];
  conferenceHours: readonly number[];
}

const side = (country: PeaceTransferCountry): TransferSide => ({ key: countryIdentityKey(country), tag: country.tag, hour: country.hour });
const tagSide = (tag: string, hour: number): TransferSide => ({ key: JSON.stringify(['tag', tag]), tag, hour });
const provenanceKey = (state: number, before: number, after: number, from: string, to: string) => JSON.stringify([state, before, after, from, to]);
const decoded = new WeakMap<PeaceDealsReply, TransferData>();

export function decodeTransfers(reply: PeaceDealsReply): TransferData {
  const held = decoded.get(reply);
  if (held) return held;
  const all: TransferOccurrence[] = [];
  const byDeal = new Map<string, TransferOccurrence[]>();
  for (const deal of reply.deals) {
    const knownAt = peaceDealKnownAt(deal);
    const provenance = new Map<string, boolean>();
    const witness = (key: string, derived: boolean) => {
      if (!provenance.has(key) || provenance.get(key) && !derived) provenance.set(key, derived);
    };
    for (const item of deal.items) {
      const change = item.change, { beforeHour, afterHour } = item.dates;
      if (change.kind === 'stateTransfer') witness(provenanceKey(change.state, beforeHour, afterHour, countryIdentityKey(change.fromCountry), countryIdentityKey(change.toCountry)), item.derived);
      else if (change.kind === 'countryReleased' && change.toCountry && change.transfers)
        for (const transfer of change.transfers) witness(provenanceKey(transfer.state, beforeHour, afterHour, countryIdentityKey(transfer.fromCountry), countryIdentityKey(change.toCountry)), item.derived);
    }
    const own: TransferOccurrence[] = [];
    const keys = new Map<string, number>();
    const push = (occurrence: Omit<TransferOccurrence, 'key'>) => {
      const base = JSON.stringify([deal.id, occurrence.from?.key ?? null, occurrence.to.key, occurrence.state, occurrence.beforeHour, occurrence.afterHour]);
      const repeat = keys.get(base) ?? 0;
      keys.set(base, repeat + 1);
      own.push({ ...occurrence, key: repeat ? `${base}#${repeat}` : base });
    };
    const graphStates = deal.transfers.links.flatMap(link => link.states);
    for (const state of graphStates) {
      const from = side(state.fromCountry), to = side(state.toCountry);
      push({ deal: deal.id, state: state.state, from, to, beforeHour: state.dates.beforeHour, afterHour: state.dates.afterHour,
        knownAt: Math.max(state.dates.afterHour, knownAt), victoryPoints: state.victoryPoints == null ? null : BigInt(state.victoryPoints),
        victoryPointsReadHour: state.victoryPointsReadHour, released: state.released === true,
        derived: provenance.get(provenanceKey(state.state, state.dates.beforeHour, state.dates.afterHour, from.key, to.key)) ?? null });
    }
    for (const item of deal.items) {
      const change = item.change;
      if (change.kind !== 'countryReleased' || change.transfers !== undefined) continue;
      const owner = change.from.length === 1 ? change.from[0] : null;
      const known = owner == null ? undefined : graphStates.find(state => state.fromCountry.tag === owner)?.fromCountry;
      const from = owner == null ? null : known ? side(known) : tagSide(owner, item.dates.beforeHour);
      for (const state of change.states) push({ deal: deal.id, state, from, to: tagSide(change.country, item.dates.afterHour),
        beforeHour: item.dates.beforeHour, afterHour: item.dates.afterHour, knownAt: Math.max(item.dates.afterHour, knownAt),
        victoryPoints: null, victoryPointsReadHour: null, released: true, derived: item.derived });
    }
    own.sort(byDate);
    byDeal.set(deal.id, own);
    all.push(...own);
  }
  all.sort(byDate);
  const hours = [...new Set(all.flatMap(occurrence => [occurrence.afterHour, occurrence.knownAt, ...(occurrence.victoryPointsReadHour == null ? [] : [occurrence.victoryPointsReadHour])]))]
    .sort((a, b) => a - b);
  const conferenceHours = [...new Set(reply.deals.flatMap(deal => [peaceDealHour(deal), peaceDealKnownAt(deal)]))].sort((a, b) => a - b);
  const data = { reply, deals: new Map(reply.deals.map(deal => [deal.id, deal])), all, byDeal, hours, conferenceHours };
  decoded.set(reply, data);
  return data;
}
const byDate = (a: TransferOccurrence, b: TransferOccurrence) => a.afterHour - b.afterHour || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);

export const transferScope = (data: TransferData, conference: string | null): readonly TransferOccurrence[] =>
  conference == null ? data.all : data.byDeal.get(conference) ?? [];

export function transferCutoff(hours: readonly number[], hour: number): number {
  let low = 0;
  let high = hours.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (hours[middle] <= hour) low = middle + 1; else high = middle;
  }
  return low ? hours[low - 1] : -Infinity;
}

export function transfersAt(list: readonly TransferOccurrence[], through: number, from = -Infinity, wholeCampaign = true): TransferOccurrence[] {
  const shown: TransferOccurrence[] = [];
  for (const occurrence of list) {
    if (occurrence.afterHour > through) break;
    if (occurrence.afterHour < from || occurrence.knownAt > through) continue;
    shown.push(!wholeCampaign && occurrence.victoryPoints != null && (occurrence.victoryPointsReadHour == null || occurrence.victoryPointsReadHour > through)
      ? { ...occurrence, victoryPoints: null, victoryPointsPending: true } : occurrence);
  }
  return shown;
}

export interface CountryBalance {
  key: string;
  side: TransferSide;
  gained: number; lost: number;
  gainedVictoryPoints: bigint | null; lostVictoryPoints: bigint | null;
  from: ReadonlyMap<string, Counterpart>; to: ReadonlyMap<string, Counterpart>;
  unattributed: number;
  first: number; last: number;
}
export interface Counterpart { side: TransferSide; count: number }
type Building = CountryBalance & { from: Map<string, Counterpart>; to: Map<string, Counterpart> };
const plus = (sum: bigint | null, value: bigint | null) => sum == null || value == null ? null : sum + value;

export function countryBalances(list: readonly TransferOccurrence[]): CountryBalance[] {
  const rows = new Map<string, Building>();
  const row = (at: TransferSide, hour: number) => {
    let found = rows.get(at.key);
    if (!found) {
      found = { key: at.key, side: at, gained: 0, lost: 0, gainedVictoryPoints: 0n, lostVictoryPoints: 0n, from: new Map(), to: new Map(), unattributed: 0, first: hour, last: hour };
      rows.set(at.key, found);
    }
    if (at.hour > found.side.hour) found.side = at;
    found.first = Math.min(found.first, hour);
    found.last = Math.max(found.last, hour);
    return found;
  };
  const count = (into: Map<string, Counterpart>, at: TransferSide) => {
    const held = into.get(at.key);
    if (held) { held.count++; if (at.hour > held.side.hour) held.side = at; } else into.set(at.key, { side: at, count: 1 });
  };
  for (const occurrence of list) {
    const value = occurrence.victoryPoints;
    const taker = row(occurrence.to, occurrence.afterHour);
    taker.gained++;
    taker.gainedVictoryPoints = plus(taker.gainedVictoryPoints, value);
    if (!occurrence.from) { taker.unattributed++; continue; }
    count(taker.from, occurrence.from);
    const giver = row(occurrence.from, occurrence.afterHour);
    giver.lost++;
    giver.lostVictoryPoints = plus(giver.lostVictoryPoints, value);
    count(giver.to, occurrence.to);
  }
  return [...rows.values()];
}
export const netVictoryPoints = (row: Pick<CountryBalance, 'gainedVictoryPoints' | 'lostVictoryPoints'>) =>
  row.gainedVictoryPoints == null || row.lostVictoryPoints == null ? null : row.gainedVictoryPoints - row.lostVictoryPoints;

export function involving(list: readonly TransferOccurrence[], key: string): TransferOccurrence[] {
  return list.filter(occurrence => occurrence.to.key === key || occurrence.from?.key === key);
}

export interface TransferHow { released: boolean; subjectOf: string | null; action: string | null }
export function transferHow(occurrence: TransferOccurrence, deal: PeaceDeal | undefined): TransferHow {
  const to = occurrence.to.tag;
  if (!deal) return { released: occurrence.released, subjectOf: null, action: null };
  let subjectOf: string | null = null;
  for (const item of deal.items) if (item.change.kind === 'newSubject' && item.change.subject === to) subjectOf = item.change.overlord;
  subjectOf ??= deal.captured?.subjects.find(subject => subject.subjectTag === to)?.overlordTag ?? null;
  const actions = deal.captured?.actions ?? [];
  const action = actions.find(action => action.name != null && action.state === occurrence.state && (action.takerTag == null || action.takerTag === to))
    ?? (occurrence.released ? actions.find(action => action.name != null && action.nameCountry == null && action.countryOperandTag === to) : undefined);
  return { released: occurrence.released, subjectOf, action: action?.name ?? null };
}

export function transferDate(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date) : formatDateTime(hour);
}
export const transferIsBy = (occurrence: Pick<TransferOccurrence, 'derived' | 'beforeHour' | 'afterHour'>) =>
  occurrence.derived !== false && occurrence.beforeHour !== occurrence.afterHour;

export function transferGaps(reply: PeaceDealsReply, from: number, to: number): HourSpan[] {
  const sorted = [...reply.unobserved.settlements, ...reply.unobserved.ownership, ...reply.unobserved.wars]
    .filter(span => span.firstHour <= to && span.lastHour >= from)
    .map(span => ({ firstHour: Math.max(from, span.firstHour), lastHour: Math.min(to, span.lastHour) }))
    .sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  const spans: HourSpan[] = [];
  for (const span of sorted) {
    const last = spans.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else spans.push({ ...span });
  }
  return spans;
}
export function transferGapHint(gaps: readonly HourSpan[]): string | null {
  if (!gaps.length) return null;
  const between = `between ${formatDateTime(gaps[0].firstHour)} and ${formatDateTime(gaps.at(-1)!.lastHour)}`;
  return gaps.length === 1 ? `Not fully recorded ${between}.` : `${gaps.length} stretches not fully recorded ${between}.`;
}
export function transferGapWindow(conference: PeaceDeal | null, list: readonly TransferOccurrence[], firstHour: number, moment: number): [number, number] {
  if (!conference) return [firstHour, moment];
  let first = conference.firstObservedHour;
  for (const occurrence of list) first = Math.min(first, occurrence.beforeHour);
  const completed = conference.captured != null || conference.endHour != null || conference.firstFinalizedHour != null;
  return [first, completed ? Math.min(moment, peaceDealHour(conference)) : moment];
}

export const transferEmptyMessage = (filtered: boolean) => filtered ? 'No territory transfers for these countries yet.' : 'No territory transfers yet.';

export function visibleConferences(deals: readonly PeaceDeal[], hour: number, from = -Infinity): PeaceDeal[] {
  return deals.filter(deal => peaceDealHour(deal) >= from && peaceDealKnownAt(deal) <= hour);
}

export function chosenConference(deals: readonly PeaceDeal[], id: string): PeaceDeal | null {
  return id ? deals.find(deal => deal.id === id) ?? null : deals[0] ?? null;
}

export function transferConferenceLabel(deal: PeaceDeal): string {
  return `${peaceDealIsBy(deal) ? 'by ' : ''}${formatDateTime(peaceDealHour(deal))} · ${peaceWarName(deal)}`;
}
