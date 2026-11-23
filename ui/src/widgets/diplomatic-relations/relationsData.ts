import type { HolderAt } from '../../dashboard/holderLooks';
import type { CountryGroupReply, DiplomaticRelationsReply, ScoreboardReply } from '../../generated/dashboard';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { NO_ROW, NOT_READ } from '../../dashboard/follow/liveCells.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { countryDrillSpans } from '../../dashboard/countryIdentity.ts';
import type { CountryDrillHistory } from '../../dashboard/countryIdentity';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import { runAt } from '../../charts/series.ts';

export type OpinionCell = { value: number } | { unknown: true; reason: string } | null;

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const hourOrNull = (value: unknown) => value === null || Number.isSafeInteger(value);

export const replyTopTags = (reply: ScoreboardReply): string[] => reply.rows.filter(row => row.cells[0] && row.cells[0].valueRaw !== null).map(row => row.tag);

export function validateRelations(data: unknown): DiplomaticRelationsReply {
  if (!object(data) || !Array.isArray(data.warnings) || !data.warnings.every(item => typeof item === 'string') || !Array.isArray(data.owners)
    || !data.owners.every(owner => object(owner) && typeof owner.tag === 'string' && hourOrNull(owner.countryId) && typeof owner.known === 'boolean'
      && (owner.reason === null || typeof owner.reason === 'string')))
    throw new Error('The diplomatic relations reply is malformed.');
  return data as unknown as DiplomaticRelationsReply;
}

export function opinionIndex(reply: DiplomaticRelationsReply, values: LiveValues<number>) {
  const owners = new Map(reply.owners.map(owner => [owner.tag, owner]));
  return (owner: string, target: string): OpinionCell => {
    const entry = owners.get(owner);
    if (!entry || !owners.has(target)) return null;
    if (!entry.known) return { unknown: true, reason: entry.reason ?? 'Not recorded' };
    const key = liveKey(owner, target), value = values.get(key);
    if (value !== undefined) return { value };
    const reason = values.missing(key);
    return reason === NO_ROW ? { value: 0 } : { unknown: true, reason };
  };
}

export function readHourOf(values: LiveValues<number>, owner: string): number | null {
  return values.get(liveKey(owner, 'readHour')) ?? null;
}

export interface OpinionDomain { low: number; high: number; settled: boolean }

export function opinionDomain(reply: DiplomaticRelationsReply, values: LiveValues<number>, held: OpinionDomain | null = null): OpinionDomain {
  if (held?.settled) return held;
  const at = opinionIndex(reply, values);
  let low = 0, high = 0, settled = true;
  for (const owner of reply.owners) for (const target of reply.owners) {
    const found = at(owner.tag, target.tag);
    if (found && 'value' in found) { low = Math.min(low, found.value); high = Math.max(high, found.value); }
    else if (found && owner.known && found.reason === NOT_READ) settled = false;
  }
  return held && held.low === low && held.high === high && held.settled === settled ? held : { low, high, settled };
}

export const relationsStructure = (reply: DiplomaticRelationsReply): string => JSON.stringify([reply.owners, reply.warnings]);

const collator = new Intl.Collator(undefined, { numeric: true });

export function orderCountries(tags: readonly string[], order: 'faction' | 'name', name: (tag: string) => string,
  groups: CountryGroupReply | undefined, hour: number, factionName: (id: string) => string): string[] {
  const factionOf = new Map<string, string>();
  if (order === 'faction' && groups) for (const faction of groups.factions) for (const run of faction.members)
    if (run.firstHour <= hour && hour <= run.lastHour && !factionOf.has(run.tag)) factionOf.set(run.tag, faction.id);
  return [...tags].sort((a, b) => {
    if (order === 'faction') {
      const [fa, fb] = [factionOf.get(a), factionOf.get(b)];
      if (fa !== fb) return fa === undefined ? 1 : fb === undefined ? -1 : collator.compare(factionName(fa), factionName(fb)) || collator.compare(fa, fb);
    }
    return collator.compare(name(a), name(b)) || collator.compare(a, b);
  });
}

export const opinionText = (value: number) => `${value > 0 ? '+' : ''}${value}`;

export interface ChosenPair { first: string; second: string; hour: number; firstId: number; secondId: number }

export function pairOf(value: unknown): ChosenPair | null {
  if (typeof value !== 'string' || !value) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    return Array.isArray(parsed) && parsed.length === 5 && typeof parsed[0] === 'string' && typeof parsed[1] === 'string'
      && [parsed[2], parsed[3], parsed[4]].every(value => Number.isSafeInteger(value))
      ? { first: parsed[0], second: parsed[1], hour: parsed[2], firstId: parsed[3], secondId: parsed[4] } : null;
  } catch { return null; }
}
export const pairToken = (pair: ChosenPair) => JSON.stringify([pair.first, pair.second, pair.hour, pair.firstId, pair.secondId]);

export const pairKeys = (pair: ChosenPair) => [{ tag: pair.first, part: String(pair.secondId) }, { tag: pair.second, part: String(pair.firstId) }];

export function heldPairKeys(identities: readonly CountryDrillHistory[], pair: ChosenPair) {
  const held = (tag: string) => {
    const identity = runAt(identities.find(history => history.tag === tag)?.identities ?? [], pair.hour);
    return identity ? { key: countryKey(identity.slot), spans: [...countryDrillSpans(identities, tag, pair.hour).values()].flat().sort((a, b) => a.firstHour - b.firstHour) } : null;
  };
  const first = held(pair.first), second = held(pair.second);
  if (!first || !second) return null;
  const keys = [{ tag: first.key, part: String(pair.secondId) }, { tag: second.key, part: String(pair.firstId) }];
  return { keys, within: new Map([[keyId(keys[0]), first.spans], [keyId(keys[1]), second.spans]]) };
}

export const pairHolders = (pair: ChosenPair): HolderAt[] => [{ key: 'first', tag: pair.first, hour: pair.hour }, { key: 'second', tag: pair.second, hour: pair.hour }];
