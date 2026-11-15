import type { HolderAt } from '../../dashboard/holderLooks';
import type { CountryGroupReply, DiplomaticRelationsReply, ScoreboardReply } from '../../generated/dashboard';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { NO_ROW, NOT_READ } from '../../dashboard/follow/liveCells.ts';

export type OpinionCell = { value: number } | { unknown: true; reason: string } | null;

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const hourOrNull = (value: unknown) => value === null || Number.isSafeInteger(value);

/** A top N's countries from a `scoreboard` reply asked for its one ranking column: greatest first, only those ranked
 * by a recorded value. */
export const replyTopTags = (reply: ScoreboardReply): string[] => reply.rows.filter(row => row.cells[0] && row.cells[0].valueRaw !== null).map(row => row.tag);

/** Checks a `diplomatic-relations` reply's shape, so a reply from another version is refused rather than read wrongly. */
export function validateRelations(data: unknown): DiplomaticRelationsReply {
  if (!object(data) || !Array.isArray(data.warnings) || !data.warnings.every(item => typeof item === 'string') || !Array.isArray(data.owners)
    || !data.owners.every(owner => object(owner) && typeof owner.tag === 'string' && hourOrNull(owner.countryId) && typeof owner.known === 'boolean'
      && (owner.reason === null || typeof owner.reason === 'string')))
    throw new Error('The diplomatic relations reply is malformed.');
  return data as unknown as DiplomaticRelationsReply;
}

/** Opinion lookup for the matrix: the row owner's opinion of the column country, from the live cells (`values`) of the
 * reply's known owners. A known owner's pair with no row holds exact zero; any other missing value, and an unknown
 * owner's whole row, is unknown with its reason. Only the reply's own countries are looked up. The diagonal is recorded
 * state like any other pair. */
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

/** The hour a known owner's opinions were last read, from its live cell; null where not known. */
export function readHourOf(values: LiveValues<number>, owner: string): number | null {
  return values.get(liveKey(owner, 'readHour')) ?? null;
}

/** The colour domain: its low and high, and whether every known owner's cells had answered when it was taken. */
export interface OpinionDomain { low: number; high: number; settled: boolean }

/** The colour domain: the recorded values' own extremes among the reply's countries, never a fixed range. Taken again
 * from `values` until every known owner's cells have answered once (a pushed panel lands before its cells do), then
 * `held` until the matrix's countries change, so the colours and the legend do not move each frame. `held` comes back
 * itself while nothing it says changed. */
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

/** What renders: the matrix's countries and whether each owner was read. Opinions and read hours are live cells. */
export const relationsStructure = (reply: DiplomaticRelationsReply): string => JSON.stringify([reply.owners, reply.warnings]);

const collator = new Intl.Collator(undefined, { numeric: true });

/** Rows and columns share one order: by faction (factions by name, countries outside any
 * faction last), then by country name; or by name alone. */
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

/** A signed opinion as the game shows it. */
export const opinionText = (value: number) => `${value > 0 ? '+' : ''}${value}`;

/** A chosen pair: two tags, the hour they named those countries, so a later holder of
 * either tag never takes its place, and those countries' raw ids, which a pair's
 * `relation-opinions` lines name the other by. */
export interface ChosenPair { first: string; second: string; hour: number; firstId: number; secondId: number }

/** The saved pair of a pinned pair history. */
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

/** The pair's two `relation-opinions` lines: each country's opinion of the other. */
export const pairKeys = (pair: ChosenPair) => [{ tag: pair.first, part: String(pair.secondId) }, { tag: pair.second, part: String(pair.firstId) }];

/** The pair's two holders, each read at the hour the pair was chosen. */
export const pairHolders = (pair: ChosenPair): HolderAt[] => [{ key: 'first', tag: pair.first, hour: pair.hour }, { key: 'second', tag: pair.second, hour: pair.hour }];
