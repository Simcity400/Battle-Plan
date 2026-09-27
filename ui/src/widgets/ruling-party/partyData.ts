import type { EventBadge } from '../../charts/badges';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import type { FixedRun, HourSpan, LeaderRun, PartyHistoryReply, PartySeries } from '../../generated/dashboard';

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
 * entry, as signed off in mock-ups politics-1/2), the others in content order. */
export function rulingFirst(list: readonly Band[], ruling: string | null): Band[] {
  const index = list.findIndex(band => band.token === ruling);
  return index <= 0 ? [...list] : [list[index], ...list.slice(0, index), ...list.slice(index + 1)];
}

/** The exact recorded popularity at an hour, or null when it was not recorded. */
export function rawAt(series: PartySeries, hour: number): string | null {
  let low = 0;
  let high = series.runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = series.runs[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return run.valueRaw;
  }
  return null;
}

export const formatPopularity = (raw: string, decimals: number) => `${formatFixed(raw, decimals, 1)}%`;

/** The ruling party's token at an hour, or null when not recorded. */
export function rulingAt(reply: PartyHistoryReply, hour: number): string | null {
  return reply.ruling.find(run => run.firstHour <= hour && hour <= run.lastHour)?.token ?? null;
}

/** Every hour where any band's value can change, with each band's stacked top
 * there; null where a band is unrecorded, and a break before an hour where a band
 * starts a new line (a new country identity). Built once per reply. */
export interface StackModel { hours: number[]; tops: (number[] | null)[]; breakBefore: boolean[] }

export function buildStack(list: readonly Band[], decimals: number): StackModel {
  const boundaries = new Set<number>();
  for (const band of list) {
    for (const run of band.series.runs) {
      boundaries.add(run.firstHour);
      boundaries.add(run.lastHour);
      // The hour after a run is sampled too, so a gap there breaks the bands.
      boundaries.add(run.lastHour + 1);
    }
  }
  const hours = [...boundaries].sort((a, b) => a - b);
  const model: StackModel = { hours, tops: [], breakBefore: [] };
  for (const hour of hours) {
    let total = 0;
    let known = true;
    let fresh = false;
    const tops: number[] = [];
    for (const band of list) {
      const run = runAt(band.series, hour);
      if (!run) { known = false; break; }
      if (run.firstHour === hour && run.startsSegment) fresh = true;
      total += fixedNumber(run.valueRaw, decimals);
      tops.push(total);
    }
    model.tops.push(known ? tops : null);
    model.breakBefore.push(fresh);
  }
  return model;
}

function runAt(series: PartySeries, hour: number): FixedRun | null {
  let low = 0;
  let high = series.runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = series.runs[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return run;
  }
  return null;
}

/** The segments to draw between `from` and `to`: samples closer than `minHours`
 * merge, but every segment keeps its first and last sample, and an isolated
 * recorded hour draws as one hour wide. */
export function stackedSegments(model: StackModel, from: number, to: number, minHours: number): { hours: number[]; tops: number[][] }[] {
  const segments: { hours: number[]; tops: number[][] }[] = [];
  let current: { hours: number[]; tops: number[][] } | null = null;
  let pending: { hour: number; tops: number[] } | null = null;
  const push = (hour: number, tops: number[]) => {
    current!.hours.push(hour);
    tops.forEach((top, index) => current!.tops[index].push(top));
  };
  const close = () => {
    if (current && pending) push(pending.hour, pending.tops);
    // One recorded hour holds its value until the next.
    if (current && current.hours.length === 1) push(current.hours[0] + 1, current.tops.map(tops => tops[0]));
    current = null;
    pending = null;
  };
  // Start from the sample at or before `from`, so the visible start is covered.
  let start = 0;
  for (let low = 0, high = model.hours.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (model.hours[middle] <= from) { start = middle; low = middle + 1; } else high = middle - 1;
  }
  for (let index = start; index < model.hours.length && model.hours[index] <= to; index++) {
    const hour = model.hours[index];
    const tops = model.tops[index];
    if (!tops || model.breakBefore[index]) close();
    if (!tops) continue;
    if (!current) {
      current = { hours: [], tops: tops.map(() => []) };
      segments.push(current);
      push(hour, tops);
      continue;
    }
    if (hour - current.hours.at(-1)! < minHours) { pending = { hour, tops }; continue; }
    push(hour, tops);
    pending = null;
  }
  close();
  return segments;
}

/** Ruling-party changes and elections as badges, and changes of leader when
 * `leaders` is set. A change next to a gap is a gap in the recording, not a change
 * of government or of leader; a leader who is not identified is no change either. */
export function partyBadges(reply: PartyHistoryReply, name: (token: string) => string, leaders = false): EventBadge[] {
  const hidden = (hour: number, unobserved: readonly HourSpan[]) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const badges: EventBadge[] = [];
  reply.ruling.forEach((run, index) => {
    const previous = reply.ruling[index - 1];
    // A run after a gap or a new country identity is not a change of government.
    if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.token !== run.token && !hidden(run.firstHour - 1, reply.unobserved)) {
      badges.push({ hour: run.firstHour, glyph: '⇆', label: `${name(run.token)} took power from ${name(previous.token)}` });
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
