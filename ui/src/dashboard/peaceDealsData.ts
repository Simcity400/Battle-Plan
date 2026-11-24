import { countryIdentityKey } from './countryIdentity.ts';
import type { PeaceDeal, PeaceDealsReply, PeaceTransferGraph, PeaceTransferredState } from '../generated/dashboard';

export const NAME_COUNTRY = '{COUNTRY}';
export const peaceActionLabel = (name: string, country: string | null): string => !name.includes(NAME_COUNTRY) ? name
  : country != null ? name.replaceAll(NAME_COUNTRY, country) : name.replaceAll(NAME_COUNTRY, '').replace(/\s+/g, ' ').trim();

const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v != null && !Array.isArray(v);
const text = (v: unknown) => typeof v === 'string';
const nullable = (v: unknown, check: (v: unknown) => boolean) => v === null || check(v);
const integer = (v: unknown) => Number.isSafeInteger(v);
const unsigned = (v: unknown) => integer(v) && (v as number) >= 0 && (v as number) <= 0xffffffff;
const bool = (v: unknown) => typeof v === 'boolean';
const decimal = (v: unknown) => text(v) && /^-?\d+$/.test(v as string);
const list = (v: unknown, check: (v: unknown) => boolean) => Array.isArray(v) && v.every(check);
const dates = (v: unknown) => object(v) && integer(v.beforeHour) && integer(v.afterHour) && (v.beforeHour as number) <= (v.afterHour as number);
const span = (v: unknown) => object(v) && integer(v.firstHour) && integer(v.lastHour) && (v.firstHour as number) <= (v.lastHour as number);
const actionKinds = new Set(['stateTransfer', 'newSubject', 'governmentChange', 'countryReleased']);
const action = (v: unknown) => object(v) && actionKinds.has(v.kind as string) && nullable(v.key, text) && nullable(v.name, text);
const government = (v: unknown) => object(v) && nullable(v.rulingParty, unsigned) && nullable(v.partyToken, text) && nullable(v.leaderIdeology, text);
function change(v: unknown): boolean {
  if (!object(v)) return false;
  switch (v.kind) {
    case 'stateTransfer': return unsigned(v.state) && text(v.from) && text(v.to) && country(v.fromCountry) && country(v.toCountry) && object(v.fromCountry) && object(v.toCountry) && v.fromCountry.tag === v.from && v.toCountry.tag === v.to && nullable(v.victoryPoints, decimal) && nullable(v.victoryPointsReadHour, integer);
    case 'newSubject': return text(v.overlord) && text(v.subject) && text(v.autonomyState) && integer(v.startHour);
    case 'governmentChange': return text(v.country) && government(v.before) && government(v.after);
    case 'countryReleased': return text(v.country) && list(v.from, text) && list(v.states, unsigned) && releasedStates(v);
    default: return false;
  }
}
function releasedStates(v: Record<string, unknown>): boolean {
  if (v.toCountry === undefined && v.transfers === undefined) return true;
  if (!country(v.toCountry) || (v.toCountry as { tag: unknown }).tag !== v.country || !Array.isArray(v.transfers)) return false;
  const states = v.states as number[], from = new Set(v.from as string[]);
  return v.transfers.length === states.length && v.transfers.every((t, index) => object(t) && t.state === states[index] && country(t.fromCountry)
    && from.has((t.fromCountry as { tag: string }).tag) && nullable(t.victoryPoints, decimal) && nullable(t.victoryPointsReadHour, integer));
}
const country = (v: unknown) => object(v) && unsigned(v.slot) && unsigned(v.countryId) && text(v.tag) && integer(v.hour);
const item = (v: unknown) => object(v) && bool(v.derived) && dates(v.dates) && nullable(v.winner, text) && action(v.action) && change(v.change);
const state = (v: unknown) => object(v) && unsigned(v.state) && dates(v.dates) && country(v.fromCountry) && country(v.toCountry) && nullable(v.victoryPoints, decimal) && nullable(v.victoryPointsReadHour, integer)
  && (v.released === undefined || bool(v.released));

export function checkedPeaceVictoryPoints(states: Iterable<Pick<PeaceTransferredState, 'victoryPoints'>>): bigint | null {
  const minimum = -(1n << 63n);
  const maximum = (1n << 63n) - 1n;
  let total = 0n;
  for (const state of states) {
    if (state.victoryPoints == null) return null;
    const value = BigInt(state.victoryPoints);
    if (value < minimum || value > maximum) return null;
    total += value;
    if (total < minimum || total > maximum) return null;
  }
  return total;
}

function graph(v: unknown): v is PeaceTransferGraph {
  if (!object(v) || !Array.isArray(v.nodes) || !Array.isArray(v.links)) return false;
  const nodes = new Map<string, { side: string; key: string }>();
  for (const n of v.nodes) {
    if (!object(n) || !text(n.id) || !country(n.country) || !['source', 'target'].includes(n.side as string) || nodes.has(n.id as string)) return false;
    nodes.set(n.id as string, { side: n.side as string, key: countryIdentityKey(n.country as { slot: number; countryId: number }) });
  }
  const ids = new Set<string>();
  for (const l of v.links) {
    if (!object(l) || !text(l.id) || ids.has(l.id as string) || nodes.get(l.source as string)?.side !== 'source' || nodes.get(l.target as string)?.side !== 'target'
      || !unsigned(l.stateCount) || l.stateCount === 0 || !list(l.states, state) || (l.states as unknown[]).length !== l.stateCount || !nullable(l.victoryPoints, decimal)) return false;
    for (const state of l.states as PeaceTransferredState[]) {
      if (countryIdentityKey(state.fromCountry) !== nodes.get(l.source as string)?.key || countryIdentityKey(state.toCountry) !== nodes.get(l.target as string)?.key) return false;
    }
    ids.add(l.id as string);
    const sum = checkedPeaceVictoryPoints(l.states as { victoryPoints: string | null }[]);
    if (l.victoryPoints !== (sum?.toString() ?? null)) return false;
  }
  return true;
}
function captured(v: unknown): boolean {
  return object(v) && integer(v.hour) && integer(v.publicationHour) && list(v.winners, unsigned) && list(v.losers, unsigned)
    && list(v.actions, a => object(a) && bool(a.derived) && ['country', 'turn', 'ordinal', 'token', 'taker', 'giver', 'negotiator', 'form'].every(k => unsigned(a[k]))
      && ['status', 'cost'].every(k => integer(a[k])) && ['tokenName', 'nameKey', 'name', 'nameCountry', 'countryTag', 'takerTag', 'giverTag', 'negotiatorTag', 'countryOperandTag', 'puppetAuxiliary', 'ship', 'ratio'].every(k => nullable(a[k], text))
      && nullable(a.state, integer) && nullable(a.countryOperand, unsigned) && nullable(a.flags, f => Array.isArray(f) && f.length === 4 && f.every(unsigned)))
    && list(v.transfers, t => object(t) && bool(t.derived) && integer(t.hour) && integer(t.state) && unsigned(t.taker) && nullable(t.to, text) && nullable(t.giver, unsigned) && nullable(t.from, text) && nullable(t.fromCountry, country) && nullable(t.toCountry, country) && bool(t.automatic))
    && list(v.subjects, s => object(s) && bool(s.derived) && unsigned(s.overlord) && unsigned(s.subject) && nullable(s.overlordTag, text) && nullable(s.subjectTag, text));
}

export function validatePeaceDealsReply(data: unknown): PeaceDealsReply {
  const observations = new Set(['recordedConference', 'conferenceNotRecorded', 'conferenceUnknown']);
  const unobserved = object(data) ? data.unobserved : null;
  const ok = object(data) && text(data.branchId) && bool(data.derived) && list(data.warnings, text) && graph(data.transfers)
    && list(data.deals, d => object(d) && text(d.id) && bool(d.derived) && observations.has(d.observation as string) && nullable(d.conferenceId, text) && nullable(d.warName, text)
      && ['firstObservedHour', 'lastObservedHour'].every(k => integer(d[k])) && ['currentTurn', 'firstFinalizedHour', 'endHour', 'firstAbsentHour'].every(k => nullable(d[k], integer))
      && nullable(d.finalized, bool) && bool(d.endIsBy) && bool(d.participantsDerived) && list(d.winners, text) && list(d.losers, text) && list(d.items, item)
      && nullable(d.captured, captured) && nullable(d.captureUnavailable, text)
      && ['captureObservedHour', 'captureUnavailableHour'].every(key => !(key in d) || nullable(d[key], integer)) && graph(d.transfers) && list(d.limitations, text))
    && object(unobserved) && ['settlements', 'conferences', 'ownership', 'wars', 'subjects', 'governments', 'existence', 'victoryPoints'].every(k => list(unobserved[k], span));
  if (!ok) throw new Error('The peace deals reply is malformed.');
  const reply = data as unknown as PeaceDealsReply;
  const ids = new Set<string>();
  for (const deal of reply.deals) {
    if (ids.has(deal.id)) throw new Error('The peace deals reply has duplicate conferences.'); ids.add(deal.id);
  }
  return reply;
}

export function peaceDealHour(deal: PeaceDeal): number {
  if (deal.captured) return deal.captured.hour;
  let hour = deal.endHour ?? deal.firstFinalizedHour ?? deal.firstObservedHour;
  for (const item of deal.items) hour = Math.max(hour, item.dates.afterHour);
  return hour;
}
type PeaceObservationTiming = { captureObservedHour?: number | null; captureUnavailableHour?: number | null };
export function peaceDealKnownAt(deal: PeaceDeal & PeaceObservationTiming): number {
  const hour = Math.max(peaceDealHour(deal), deal.captureObservedHour ?? -Infinity);
  return deal.captured ? Math.max(hour, deal.captured.publicationHour)
    : deal.endHour != null ? Math.max(hour, deal.firstAbsentHour ?? hour) : hour;
}
export function peaceDealAt(deal: PeaceDeal & PeaceObservationTiming, hour: number): PeaceDeal {
  return deal.captureUnavailable != null && (deal.captureUnavailableHour ?? -Infinity) > hour
    ? { ...deal, captureUnavailable: null, limitations: deal.limitations.filter(reason => reason !== deal.captureUnavailable) } : deal;
}
export function peaceDealIsBy(deal: PeaceDeal): boolean {
  return !deal.captured && (deal.endIsBy || deal.items.some(item => item.dates.beforeHour !== item.dates.afterHour));
}
