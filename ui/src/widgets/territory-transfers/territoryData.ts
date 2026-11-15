import type { HourSpan } from '../../generated/dashboard';
import { peaceDealHour, peaceDealIsBy, peaceDealKnownAt, peaceWarName } from '../../dashboard/peaceDealsData.ts';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import type { PeaceDeal, PeaceDealsReply, PeaceTransferCountry } from '../../generated/dashboard';

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

/** One end of a transfer: its native holder and the read that names it. */
export interface TransferSide { key: string; tag: string; hour: number }
/** One state changing hands once; a state transferred twice is two occurrences. */
export interface TransferOccurrence {
  key: string; deal: string; state: number;
  /** Null only where an older bridge names several former owners for one release. */
  from: TransferSide | null; to: TransferSide;
  beforeHour: number; afterHour: number;
  /** The supplying observation, independently of the outcome's actual date. */
  knownAt: number;
  victoryPoints: bigint | null; victoryPointsReadHour: number | null;
  /** Live has not reached this state's own victory-points read. */
  victoryPointsPending?: boolean;
  /** The same settlement released the taker. */
  released: boolean;
  /** An exact finalizer outcome (false), an ownership comparison (true), or no matching item. */
  derived: boolean | null;
}
export interface TransferData {
  reply: PeaceDealsReply;
  deals: ReadonlyMap<string, PeaceDeal>;
  /** Every occurrence by its date; a deal's own list keeps the same order. */
  all: readonly TransferOccurrence[];
  byDeal: ReadonlyMap<string, readonly TransferOccurrence[]>;
  /** Dates, supplying observations and VP reads, so playback ticks reuse one cut. */
  hours: readonly number[];
  conferenceHours: readonly number[];
}

const side = (country: PeaceTransferCountry): TransferSide => ({ key: countryIdentityKey(country), tag: country.tag, hour: country.hour });
/** A holder an older bridge names only by tag; never joined to a native identity. */
const tagSide = (tag: string, hour: number): TransferSide => ({ key: JSON.stringify(['tag', tag]), tag, hour });
const provenanceKey = (state: number, before: number, after: number, from: string, to: string) => JSON.stringify([state, before, after, from, to]);
const decoded = new WeakMap<PeaceDealsReply, TransferData>();

/** Every deal's graph occurrence, joined to its deal and item provenance once. A
 * reply from a bridge built before released states joined the graphs keeps them
 * from the release item, without victory points. */
export function decodeTransfers(reply: PeaceDealsReply): TransferData {
  const held = decoded.get(reply);
  if (held) return held;
  const all: TransferOccurrence[] = [];
  const byDeal = new Map<string, TransferOccurrence[]>();
  for (const deal of reply.deals) {
    const knownAt = peaceDealKnownAt(deal);
    const provenance = new Map<string, boolean>();
    const witness = (key: string, derived: boolean) => {
      // An exact finalizer item supersedes a matching derived observation.
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
      // Only a single former owner can be named for each state; the giver lost
      // other states in this settlement, so the graph holds its identity.
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

/** The campaign's occurrences, or one conference's. */
export const transferScope = (data: TransferData, conference: string | null): readonly TransferOccurrence[] =>
  conference == null ? data.all : data.byDeal.get(conference) ?? [];

/** Last hour at which anything shown can change, so playback ticks between them
 * reuse the same lists without another pass. */
export function transferCutoff(hours: readonly number[], hour: number): number {
  let low = 0;
  let high = hours.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (hours[middle] <= hour) low = middle + 1; else high = middle;
  }
  return low ? hours[low - 1] : -Infinity;
}

/** Occurrences reached by `through`, dated from `from` (Range includes both ends).
 * Unless the whole campaign is shown, a state's VP waits for its own read. */
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

/** A country's peace-settlement territory: states received and given up, with their
 * counterparts. Missing VPs make that side unknown; no partial sum becomes known. */
export interface CountryBalance {
  key: string;
  /** The newest read naming this holder among the shown transfers. */
  side: TransferSide;
  gained: number; lost: number;
  gainedVictoryPoints: bigint | null; lostVictoryPoints: bigint | null;
  from: ReadonlyMap<string, Counterpart>; to: ReadonlyMap<string, Counterpart>;
  /** Received states whose former owner is not named. */
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

/** Transfers involving one holder, or the old pinned giver → taker links. */
export function involving(list: readonly TransferOccurrence[], key: string): TransferOccurrence[] {
  return list.filter(occurrence => occurrence.to.key === key || occurrence.from?.key === key);
}

/** How the recorded settlement moved the state, as far as it records. */
export interface TransferHow { released: boolean; subjectOf: string | null; action: string | null }
export function transferHow(occurrence: TransferOccurrence, deal: PeaceDeal | undefined): TransferHow {
  const to = occurrence.to.tag;
  if (!deal) return { released: occurrence.released, subjectOf: null, action: null };
  let subjectOf: string | null = null;
  for (const item of deal.items) if (item.change.kind === 'newSubject' && item.change.subject === to) subjectOf = item.change.overlord;
  subjectOf ??= deal.captured?.subjects.find(subject => subject.subjectTag === to)?.overlordTag ?? null;
  const actions = deal.captured?.actions ?? [];
  // The content's own action name, never a guessed one; a demand naming its country (a puppet's) releases none.
  const action = actions.find(action => action.name != null && action.state === occurrence.state && (action.takerTag == null || action.takerTag === to))
    ?? (occurrence.released ? actions.find(action => action.name != null && action.nameCountry == null && action.countryOperandTag === to) : undefined);
  return { released: occurrence.released, subjectOf, action: action?.name ?? null };
}

/** Plain dates in rows; the tooltip carries the hour and any "by" caveat. */
export function transferDate(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date) : formatDateTime(hour);
}
/** An ownership comparison bounds the change between two reads. */
export const transferIsBy = (occurrence: Pick<TransferOccurrence, 'derived' | 'beforeHour' | 'afterHour'>) =>
  occurrence.derived !== false && occurrence.beforeHour !== occurrence.afterHour;

/** Gaps in the hourly settlement, ownership and war inputs that establish transfers,
 * including hours missing from the recording. The conference roster is read once a
 * day: the hours between its reads are its cadence, not missing transfers. */
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
/** One hint for every gap in the shown time, never one line per gap. */
export function transferGapHint(gaps: readonly HourSpan[]): string | null {
  if (!gaps.length) return null;
  const between = `between ${formatDateTime(gaps[0].firstHour)} and ${formatDateTime(gaps.at(-1)!.lastHour)}`;
  return gaps.length === 1 ? `Not fully recorded ${between}.` : `${gaps.length} stretches not fully recorded ${between}.`;
}
/** A completed conference's gaps belong to its own observed interval; an active one follows Live. */
export function transferGapWindow(conference: PeaceDeal | null, list: readonly TransferOccurrence[], firstHour: number, moment: number): [number, number] {
  if (!conference) return [firstHour, moment];
  let first = conference.firstObservedHour;
  for (const occurrence of list) first = Math.min(first, occurrence.beforeHour);
  const completed = conference.captured != null || conference.endHour != null || conference.firstFinalizedHour != null;
  return [first, completed ? Math.min(moment, peaceDealHour(conference)) : moment];
}

/** The body's message when nothing is shown. */
export const transferEmptyMessage = (filtered: boolean) => filtered ? 'No territory transfers for these countries yet.' : 'No territory transfers yet.';

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

/** Event bounds and the war name, for the conference picker and the scope line. */
export function transferConferenceLabel(deal: PeaceDeal): string {
  return `${peaceDealIsBy(deal) ? 'by ' : ''}${formatDateTime(peaceDealHour(deal))} · ${peaceWarName(deal)}`;
}
