import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { EventBadge } from '../../charts/badges';
import { GREY_CHROMA } from '../../charts/palette.ts';
import { oklchToRgb, parseColor, rgbToOklch, toHex } from '../../shell/appearance/color.ts';
import type { HourSpan, LeaderRun, PartOwnership, PartyHistoryReply } from '../../generated/dashboard';
import type { SeriesKey } from '../../dashboard/keyedSeriesData';

export const POPULARITY_FAMILY = { kind: 'party-popularity', measure: 'popularity' } as const;
export const POPULARITY_PART_BITS = 32;

export interface Band { readonly token: string; readonly name: string; readonly color: string; readonly owned: readonly PartOwnership[] }

export function validatePartyHistoryReply(data: unknown): PartyHistoryReply {
  const reply = data as PartyHistoryReply;
  const ok = typeof reply === 'object' && reply != null && Number.isInteger(reply.decimals) && Array.isArray(reply.parties)
    && reply.parties.every(party => typeof party.token === 'string' && Array.isArray(party.parts)
      && party.parts.every(owned => Number.isInteger(owned.part) && owned.part >= 0 && owned.firstHour <= owned.lastHour))
    && Array.isArray(reply.ruling) && Array.isArray(reply.elections)
    && reply.elections.every(election => Number.isInteger(election.hour) && election.hour <= election.witnessed) && Array.isArray(reply.ideologies) && Array.isArray(reply.warnings)
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

export function bands(reply: PartyHistoryReply, palette: readonly string[]): Band[] {
  const tone = paletteTone(palette);
  let fallback = 0;
  return reply.parties.map(series => {
    const ideology = reply.ideologies.find(item => item.token === series.token);
    const color = ideology?.color ? ideologyColour(ideology.color, tone) : palette[fallback++ % Math.max(1, palette.length)]!;
    return {
      token: series.token,
      name: contentTitle(ideology?.name, series.token, 'Ideology name unavailable'),
      color,
      owned: series.parts,
    };
  });
}

export function ownedPartAt(band: Band, hour: number): number | null {
  for (const owned of band.owned) if (owned.firstHour <= hour && hour <= owned.lastHour) return owned.part;
  return null;
}

export function bandKeys(list: readonly Band[], subject: string): SeriesKey[] {
  const parts = new Set<number>();
  for (const band of list) for (const owned of band.owned) parts.add(owned.part);
  return [...parts].sort((a, b) => a - b).map(part => ({ tag: subject, part: String(part) }));
}

export function paletteTone(palette: readonly string[]): { l: number; c: number } | null {
  const tones = palette.flatMap(hex => { const colour = parseColor(hex); return colour ? [rgbToOklch(colour)] : []; });
  if (!tones.length) return null;
  const median = (values: number[]) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]!;
  return { l: median(tones.map(tone => tone.l)), c: median(tones.map(tone => tone.c)) };
}

export function ideologyColour([r, g, b]: readonly [number, number, number], tone: { l: number; c: number } | null): string {
  const source = rgbToOklch({ r, g, b });
  if (!tone) return toHex({ r, g, b });
  return toHex(oklchToRgb({ l: tone.l, c: source.c < GREY_CHROMA ? source.c : Math.min(source.c, tone.c), h: source.h }));
}

export function rulingAt(reply: PartyHistoryReply, hour: number): string | null {
  return reply.ruling.find(run => run.firstHour <= hour && hour <= run.lastHour)?.token ?? null;
}

export function partyBadges(reply: PartyHistoryReply, name: (token: string) => string, hour: number, leaders = false): EventBadge[] {
  const hidden = (hour: number, unobserved: readonly HourSpan[]) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const badges: EventBadge[] = [];
  reply.ruling.forEach((run, index) => {
    const previous = reply.ruling[index - 1];
    if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.token !== run.token && !hidden(run.firstHour - 1, reply.unobserved)) {
      badges.push({ hour: run.firstHour, glyph: '⇆', label: `${name(run.token)} took power from ${name(previous.token)}` });
    }
  });
  for (const election of reply.elections) if (election.witnessed <= hour) badges.push({ hour: election.hour, glyph: '✓', label: 'Election held' });
  if (leaders) {
    reply.leaders.forEach((run, index) => {
      const previous = reply.leaders[index - 1];
      if (!previous || run.startsSegment || previous.lastHour + 1 !== run.firstHour || run.person == null || previous.person == null) return;
      if (reply.people[run.person]!.character.id === reply.people[previous.person]!.character.id) return;
      const was = leaderShown(reply, previous);
      const now = leaderShown(reply, run);
      badges.push({ hour: run.firstHour, glyph: '♔', label: now.known && was.known ? `${now.name} succeeded ${was.name} as leader` : 'The leader changed' });
    });
  }
  return badges.sort((a, b) => a.hour - b.hour);
}

export function leaderAt(reply: PartyHistoryReply, hour: number): LeaderRun | null {
  let low = 0;
  let high = reply.leaders.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = reply.leaders[middle]!;
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return run;
  }
  return null;
}

export function partyNameAt(reply: PartyHistoryReply, hour: number): string | null | undefined {
  let low = 0;
  let high = reply.partyNameRuns.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = reply.partyNameRuns[middle]!;
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return reply.partyNames[run.name];
  }
  return undefined;
}

export interface LeaderShown { name: string; known: boolean; portrait: string | null; portraitMissing: string | null; ideology: string | null }

export function leaderShown(reply: PartyHistoryReply, run: LeaderRun | null): LeaderShown {
  const none = (name: string, ideology: string | null = null): LeaderShown => ({ name, known: false, portrait: null, portraitMissing: null, ideology });
  if (!run) return none('Leader not recorded');
  if (!run.ideology) return none('No leader');
  const ideology = contentTitle(reply.ideologyTypes.find(item => item.token === run.ideology)?.name,
    run.ideology, 'Ideology name unavailable');
  const person = run.person == null ? undefined : reply.people[run.person];
  if (!person) return none('Leader not identified', ideology);
  return { name: person.name ?? 'Leader’s name not known', known: person.name != null, portrait: person.portrait, portraitMissing: person.portraitMissing, ideology };
}
