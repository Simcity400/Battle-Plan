import { causeLabel } from '../widgets/bombing-damage/militaryAirAt.ts';
import { isOrderedSpans, runAt } from '../charts/series.ts';
import type { Span } from '../charts/series';
import { object, opaqueTag, signed32, unsigned32 } from './validation.ts';
import type { MilitaryAirOpening, MilitaryAirReply } from '../generated/dashboard';

export type MilitaryAirLedger = Omit<MilitaryAirReply, 'window' | 'notes' | 'countedFrom' | 'openings' | 'unjoined' | 'causes'> & Partial<Pick<MilitaryAirReply, 'causes'>>
  & { countedFrom?: number | null; openings?: MilitaryAirOpening[] };

const signed128 = (value: bigint) => value >= -(1n << 127n) && value < (1n << 127n);
const exact128 = (value: unknown): value is string => typeof value === 'string' && /^(?:0|-?[1-9]\d*)$/.test(value)
  && (value.length < 39 || signed128(BigInt(value)));
const optional = <T>(value: unknown, valid: (value: unknown) => value is T): value is T | null => value === null || valid(value);
const text = (value: unknown): value is string => typeof value === 'string';
const bool = (value: unknown): value is boolean => typeof value === 'boolean';
const within = (span: Span, spans: readonly Span[]) => (runAt(spans, span.firstHour)?.lastHour ?? -Infinity) >= span.lastHour;

const count = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

function isOpenings(value: unknown, family: unknown): value is MilitaryAirOpening[] {
  if (!Array.isArray(value)) return false;
  const keys = new Set<string>();
  for (const opening of value as unknown[]) {
    if (!object(opening) || !optional(opening.identity, text) || opening.identity === '' || !bool(opening.dealt) || !unsigned32(opening.cause)
      || (family === 'airLosses' ? !bool(opening.opponentKnown) : opening.opponentKnown !== null) || !exact128(opening.total)) return false;
    const key = JSON.stringify([opening.identity, opening.dealt, opening.cause, opening.opponentKnown]);
    if (keys.has(key)) return false;
    keys.add(key);
  }
  return true;
}

function isMilitaryAirReply(value: unknown, windowed = false): value is MilitaryAirLedger {
  if (!object(value) || !['airLosses', 'bombing'].includes(String(value.family)) || !signed32(value.firstHour) || !signed32(value.lastHour)
    || typeof value.available !== 'boolean' || !isOrderedSpans(value.observed) || !isOrderedSpans(value.incomplete)
    || !Array.isArray(value.countries) || !Array.isArray(value.parts) || !Array.isArray(value.regions) || !Array.isArray(value.buildings)
    || !Array.isArray(value.warnings) || !value.warnings.every(text)) return false;
  if (windowed && (!unsigned32(value.window) || !Array.isArray(value.notes) || !value.notes.every(note => object(note) && text(note.reason) && count(note.events)))) return false;
  if ((windowed || value.countedFrom !== undefined) && !optional(value.countedFrom, signed32)) return false;
  if ((windowed || value.openings !== undefined) && !isOpenings(value.openings, value.family)) return false;
  if (value.firstHour > value.lastHour && !(windowed && !value.observed.length && !value.countries.length && !value.parts.length)) return false;
  const observed = value.observed, first = value.firstHour, lastHour = value.lastHour;
  const reachesBack = (span: Span, index: number) => index === 0 && span.lastHour >= first && typeof value.countedFrom === 'number' && span.firstHour >= value.countedFrom;
  if (observed.some((span, index) => (span.firstHour < first && !reachesBack(span, index)) || span.lastHour > lastHour) || value.available !== (observed.length > 0)
    || !value.incomplete.every(span => within(span, observed))) return false;
  const countries = new Map<string, Map<string, Span[]>>();
  for (const country of value.countries) {
    if (!object(country) || !opaqueTag(country.tag) || !text(country.identity) || !country.identity || !isOrderedSpans(country.present)) return false;
    const membership: unknown = country.membership;
    if (!optional(membership, isOrderedSpans)) return false;
    const identities = countries.get(country.tag) ?? new Map<string, Span[]>();
    const present = country.present;
    if (identities.has(country.identity) || !present.every(span => within(span, observed))
      || (membership as Span[] | null | undefined)?.some(span => !within(span, present))) return false;
    identities.set(country.identity, present);
    countries.set(country.tag, identities);
  }
  const keys = new Set<string>();
  for (const part of value.parts) {
    if (!object(part) || !optional(part.tag, opaqueTag) || !optional(part.identity, text) || (part.tag === null) !== (part.identity === null)
      || typeof part.dealt !== 'boolean' || !optional(part.cause, unsigned32) || !optional(part.opponentKnown, bool) || !optional(part.buildingId, unsigned32)
      || !optional(part.buildingToken, text) || !optional(part.state, unsigned32) || !Array.isArray(part.points)) return false;
    if (value.family === 'airLosses' ? part.cause === null || part.opponentKnown === null || part.buildingId !== null || part.buildingToken !== null || part.state !== null
      : part.opponentKnown !== null || (part.cause === null) !== (part.buildingId === null) || (part.buildingId === null) !== (part.state === null)) return false;
    const key = JSON.stringify([part.tag, part.identity, part.dealt, part.cause, part.opponentKnown, part.buildingId, part.buildingToken, part.state]);
    if (keys.has(key)) return false;
    keys.add(key);
    const present = part.tag === null ? observed : countries.get(part.tag)?.get(part.identity as string);
    if (!present) return false;
    let last: number | null = null, read: number | null = null;
    for (const point of part.points as unknown[]) {
      if (!Array.isArray(point) || point.length !== 3) return false;
      const [hour, readHour, total] = point as unknown[];
      if (!signed32(hour) || !signed32(readHour) || hour > readHour || !exact128(total)
        || read !== null && (readHour < read || readHour === read && last !== null && hour <= last)
        || !runAt(present, readHour)) return false;
      last = hour; read = readHour;
    }
  }
  for (const names of [value.regions, value.buildings]) {
    const ids = new Set<string>();
    for (const name of names) {
      if (!object(name) || !text(name.id) || !name.id || !optional(name.name, text) || !optional(name.missing, text)
        || (name.name === null) !== (name.missing !== null) || ids.has(name.id)) return false;
      ids.add(name.id);
    }
  }
  return true;
}

export function validateMilitaryAirWindow(value: unknown): MilitaryAirReply {
  if (!isMilitaryAirReply(value, true)) throw new Error('Invalid military air ledger reply');
  return value as unknown as MilitaryAirReply;
}

export function ledgerCauseName(reply: Pick<MilitaryAirLedger, 'causes'> | undefined, cause: number): string {
  const named = reply?.causes?.find(item => item.cause === cause);
  return named ? causeLabel(named) : `Cause ${cause}`;
}
