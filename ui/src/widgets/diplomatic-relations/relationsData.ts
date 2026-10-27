import type { Run } from '../../charts/series';
import type { HolderAt } from '../../dashboard/holderLooks';
import type { CountryGroupReply, DiplomacyOpinionsReply, DiplomacyRun, ScoreboardColumnKey, ScoreboardPreparedReply } from '../../generated/dashboard';
import { fieldPreparing, pendingReplyAt } from '../../dashboard/preparedPrefix.ts';
import { preparedScoreboardRows } from '../scoreboard/preparedScoreboard.ts';

export type OpinionCell = { value: number; readHour: number | null } | { unknown: true } | null;

/** Opinion lookup for the matrix: the row owner's opinion of the column country. A known
 * owner without a listed target holds exact zero; an unknown owner's whole row is unknown.
 * The diagonal is recorded state like any other pair. */
export function opinionIndex(reply: DiplomacyOpinionsReply) {
  const owners = new Map(reply.owners.map(owner => [owner.tag, { owner, values: new Map(owner.opinions.map(item => [item.target, item.value])) }]));
  return (owner: string, target: string): OpinionCell => {
    const entry = owners.get(owner);
    if (!entry) return null;
    if (!entry.owner.known) return { unknown: true };
    return { value: entry.values.get(target) ?? 0, readHour: entry.owner.readHour };
  };
}

/** The colour domain: the recorded values' own extremes, never a fixed range. */
export function opinionDomain(reply: DiplomacyOpinionsReply): [number, number] {
  let low = 0, high = 0;
  for (const owner of reply.owners) for (const item of owner.opinions) { low = Math.min(low, item.value); high = Math.max(high, item.value); }
  return [low, high];
}

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

/** A top N's countries from one Scoreboard reply, highest first; null while that reply's
 * hour or measure is still being prepared, which ranks nothing yet. */
export function topTags(reply: ScoreboardPreparedReply, hour: number, measure: ScoreboardColumnKey): string[] | null {
  if (pendingReplyAt(reply, hour) || fieldPreparing(reply, measure)) return null;
  return preparedScoreboardRows(reply, undefined, measure, true).map(row => row.tag);
}

/** A direction's line: recorded values only; unknown stretches are gaps. */
export const pairLine = (runs: readonly DiplomacyRun[]): Run[] => runs.flatMap((run, index) => run.value === null ? []
  : [{ firstHour: run.firstHour, lastHour: run.lastHour, value: run.value, ...(run.startsSegment || index === 0 || runs[index - 1].value === null ? { breakBefore: true } : {}) }]);

/** A signed opinion as the game shows it. */
export const opinionText = (value: number) => `${value > 0 ? '+' : ''}${value}`;

/** A chosen pair: two tags and the hour they named those countries, so a later holder of
 * either tag never takes its place. */
export interface ChosenPair { first: string; second: string; hour: number }

/** The saved pair of a pinned pair history. */
export function pairOf(value: unknown): ChosenPair | null {
  if (typeof value !== 'string' || !value) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    return Array.isArray(parsed) && parsed.length === 3 && typeof parsed[0] === 'string' && typeof parsed[1] === 'string' && Number.isSafeInteger(parsed[2])
      ? { first: parsed[0], second: parsed[1], hour: parsed[2] } : null;
  } catch { return null; }
}
export const pairToken = (pair: ChosenPair) => JSON.stringify([pair.first, pair.second, pair.hour]);

/** The pair's two holders, each read at the hour the pair was chosen. */
export const pairHolders = (pair: ChosenPair): HolderAt[] => [{ key: 'first', tag: pair.first, hour: pair.hour }, { key: 'second', tag: pair.second, hour: pair.hour }];
