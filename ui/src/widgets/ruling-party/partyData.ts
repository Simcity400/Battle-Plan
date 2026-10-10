import type { EventBadge } from '../../charts/badges';
import { runAt } from '../../charts/series.ts';
import { buildStack as buildStackOf, stackedSegments } from '../../charts/stack.ts';
import type { StackModel } from '../../charts/stack';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import type { FixedRun, HourSpan, LeaderRun, LedgerGovernment, PartyHistoryReply, PartySeries } from '../../generated/dashboard';

/** Party popularity as stacked bands in each ideology's content colour, with the
 * ruling party's changes and elections as badges. */

export interface Band { token: string; name: string; color: string; series: PartySeries }

/** Checks the reply's shape; runs must be in hour order. */
export function validatePartyHistoryReply(data: unknown): PartyHistoryReply {
  const reply = data as PartyHistoryReply;
  const ordered = (runs: readonly FixedRun[]) => runs.every((run, index) => run.firstHour <= run.lastHour
    && /^-?\d{1,20}$/.test(run.valueRaw) && (index === 0 || runs[index - 1].lastHour < run.firstHour));
  const ok = typeof reply === 'object' && reply != null && Number.isInteger(reply.decimals) && Array.isArray(reply.parties)
    && reply.parties.every(party => typeof party.token === 'string' && Array.isArray(party.runs) && ordered(party.runs))
    && Array.isArray(reply.ruling) && Array.isArray(reply.elections) && Array.isArray(reply.ideologies) && Array.isArray(reply.warnings)
    && Array.isArray(reply.people) && Array.isArray(reply.ideologyTypes) && Array.isArray(reply.leaders)
    && reply.leaders.every((run, index) => typeof run.ideology === 'string' && run.firstHour <= run.lastHour
      && (run.person == null || (Number.isInteger(run.person) && run.person >= 0 && run.person < reply.people.length))
      && (index === 0 || reply.leaders[index - 1].lastHour < run.firstHour))
    && Array.isArray(reply.partyNames) && reply.partyNames.every(name => name === null || typeof name === 'string')
    && Array.isArray(reply.partyNameRuns)
    && reply.partyNameRuns.every((run, index) => Number.isInteger(run.name) && run.name >= 0 && run.name < reply.partyNames.length
      && run.firstHour <= run.lastHour && (index === 0 || reply.partyNameRuns[index - 1].lastHour < run.firstHour));
  if (!ok) throw new Error('The party history reply is malformed.');
  return reply;
}

/** Bands in content order (unknown ideologies after, by token), coloured by content
 * or, without a content colour, by the categorical palette in its fixed order. */
export function bands(reply: PartyHistoryReply, palette: readonly string[]): Band[] {
  const order = new Map(reply.ideologies.map((ideology, index) => [ideology.token, index]));
  const sorted = [...reply.parties].sort((a, b) => (order.get(a.token) ?? Infinity) - (order.get(b.token) ?? Infinity) || a.token.localeCompare(b.token));
  let fallback = 0;
  return sorted.map(series => {
    const ideology = reply.ideologies.find(item => item.token === series.token);
    const color = ideology?.color ? `rgb(${ideology.color.join(' ')})` : palette[fallback++ % Math.max(1, palette.length)];
    return { token: series.token, name: ideology?.name ?? series.token, color, series };
  });
}

/** The ruling party's band first (the bottom of the stack and the legend's first
 * entry), the others in content order. */
export function rulingFirst(list: readonly Band[], ruling: string | null): Band[] {
  const index = list.findIndex(band => band.token === ruling);
  return index <= 0 ? [...list] : [list[index], ...list.slice(0, index), ...list.slice(index + 1)];
}

/** The exact recorded popularity at an hour, or null when it was not recorded. */
export function rawAt(series: PartySeries, hour: number): string | null {
  return runAt(series.runs, hour)?.valueRaw ?? null;
}

export const formatPopularity = (raw: string, decimals: number) => `${formatFixed(raw, decimals, 1)}%`;

/** The ruling party's token at an hour, or null when not recorded. */
export function rulingAt(reply: PartyHistoryReply, hour: number): string | null {
  return reply.ruling.find(run => run.firstHour <= hour && hour <= run.lastHour)?.token ?? null;
}

/** The bands stacked in order, each party's popularity as a plotting number
 * (the kit's `buildStack`). */
export function buildStack(list: readonly Band[], decimals: number): StackModel {
  return buildStackOf(list.map(band => band.series.runs.map(run => ({ ...run, value: fixedNumber(run.valueRaw, decimals) }))));
}

export { stackedSegments };

/** Ruling-party changes and elections as badges, and changes of leader when
 * `leaders` is set. A change next to a gap is a gap in the recording, not a change
 * of government or of leader; a leader who is not identified is no change either. */
export function partyBadges(reply: PartyHistoryReply, name: (token: string) => string, leaders = false, governments: readonly LedgerGovernment[] = []): EventBadge[] {
  const hidden = (hour: number, unobserved: readonly HourSpan[]) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const badges: EventBadge[] = [];
  reply.ruling.forEach((run, index) => {
    const previous = reply.ruling[index - 1];
    // A run after a gap or a new country identity is not a change of government.
    if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.token !== run.token && !hidden(run.firstHour - 1, reply.unobserved)) {
      // Only exact captured endpoints and their supplying read supersede census.
      const captured = governments.some(change => change.readHour === run.firstHour
        && change.beforeToken === previous.token && change.afterToken === run.token);
      if (!captured) badges.push({ hour: run.firstHour, glyph: '⇆', label: `${name(run.token)} took power from ${name(previous.token)}` });
    }
  });
  for (const hour of reply.elections) badges.push({ hour, glyph: '✓', label: 'Election held' });
  if (leaders) {
    reply.leaders.forEach((run, index) => {
      const previous = reply.leaders[index - 1];
      if (!previous || run.startsSegment || previous.lastHour + 1 !== run.firstHour || run.person == null || previous.person == null) return;
      // A rename or a new portrait is the same leader.
      if (reply.people[run.person].character.id === reply.people[previous.person].character.id) return;
      const was = leaderShown(reply, previous);
      const now = leaderShown(reply, run);
      badges.push({ hour: run.firstHour, glyph: '♔', label: now.known && was.known ? `${now.name} succeeded ${was.name} as leader` : 'The leader changed' });
    });
  }
  return badges.sort((a, b) => a.hour - b.hour);
}

/** The ruling party's leader run at an hour, or null when not recorded. */
export function leaderAt(reply: PartyHistoryReply, hour: number): LeaderRun | null {
  let low = 0;
  let high = reply.leaders.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = reply.leaders[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return run;
  }
  return null;
}

/** The ruling party's name at an hour as the game names it: undefined when it was
 * not recorded, null when it cannot be known. */
export function partyNameAt(reply: PartyHistoryReply, hour: number): string | null | undefined {
  let low = 0;
  let high = reply.partyNameRuns.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = reply.partyNameRuns[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return reply.partyNames[run.name];
  }
  return undefined;
}

/** What the header and tooltip say about the leader at an hour. `name` is what to
 * show; `known` is false when that is a notice rather than the leader's name;
 * `portraitMissing` says why there is no portrait. */
export interface LeaderShown { name: string; known: boolean; portrait: string | null; portraitMissing: string | null; ideology: string | null }

export function leaderShown(reply: PartyHistoryReply, run: LeaderRun | null): LeaderShown {
  const none = (name: string, ideology: string | null = null): LeaderShown => ({ name, known: false, portrait: null, portraitMissing: null, ideology });
  if (!run) return none('Leader not recorded');
  if (!run.ideology) return none('No leader');
  const ideology = reply.ideologyTypes.find(item => item.token === run.ideology)?.name ?? run.ideology;
  const person = run.person == null ? undefined : reply.people[run.person];
  if (!person) return none('Leader not identified', ideology);
  return { name: person.name ?? 'Leader’s name not known', known: person.name != null, portrait: person.portrait, portraitMissing: person.portraitMissing, ideology };
}
