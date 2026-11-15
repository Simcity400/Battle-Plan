import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { EventBadge } from '../../charts/badges';
import { runAt } from '../../charts/series.ts';
import { formatFixed } from '../../charts/valueAxis.ts';
import { GREY_CHROMA } from '../../charts/palette.ts';
import { oklchToRgb, parseColor, rgbToOklch, toHex } from '../../shell/appearance/color.ts';
import type { HourSpan, LeaderRun, PartyHistoryReply } from '../../generated/dashboard';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import type { ReachedKeyLine, SeriesKey } from '../../dashboard/keyedSeriesData';
import { mergeSummaries } from '../../charts/chartSummary.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import type { LedgerGovernment } from './partyTypes';

/** Party popularity as stacked bands in each ideology's colour, with the ruling
 * party's changes and elections as badges. Popularity is the declared `party-popularity` series: each band's lines
 * are the parts its party was read under. */

/** The declared series of party popularity, and the bits of its row key below a country. */
export const POPULARITY_FAMILY = { kind: 'party-popularity', measure: 'popularity' } as const;
export const POPULARITY_PART_BITS = 32;

export interface Band { token: string; name: string; color: string; keys: SeriesKey[] }

/** Checks the reply's shape; runs must be in hour order. */
export function validatePartyHistoryReply(data: unknown): PartyHistoryReply {
  const reply = data as PartyHistoryReply;
  const ok = typeof reply === 'object' && reply != null && Number.isInteger(reply.decimals) && Array.isArray(reply.parties)
    && reply.parties.every(party => typeof party.token === 'string' && Array.isArray(party.parts)
      && party.parts.every(part => Number.isInteger(part) && part >= 0))
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

/** Bands in content order (unknown ideologies after, by token), coloured by their
 * content colour in the palette's tone or, without one, by the categorical palette
 * in its fixed order. */
export function bands(reply: PartyHistoryReply, tag: string, palette: readonly string[]): Band[] {
  const order = new Map(reply.ideologies.map((ideology, index) => [ideology.token, index]));
  const sorted = [...reply.parties].sort((a, b) => (order.get(a.token) ?? Infinity) - (order.get(b.token) ?? Infinity) || a.token.localeCompare(b.token));
  const tone = paletteTone(palette);
  let fallback = 0;
  return sorted.map(series => {
    const ideology = reply.ideologies.find(item => item.token === series.token);
    const color = ideology?.color ? ideologyColour(ideology.color, tone) : palette[fallback++ % Math.max(1, palette.length)];
    return { token: series.token, name: contentTitle(ideology?.name, series.token, 'Ideology name unavailable'), color,
      keys: series.parts.map(part => ({ tag, part: String(part) })) };
  });
}

/** The categorical palette's median lightness and chroma: the tone every category
 * colour on this theme shares. Null for an empty or unreadable palette. */
export function paletteTone(palette: readonly string[]): { l: number; c: number } | null {
  const tones = palette.flatMap(hex => { const colour = parseColor(hex); return colour ? [rgbToOklch(colour)] : []; });
  if (!tones.length) return null;
  const median = (values: number[]) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
  return { l: median(tones.map(tone => tone.l)), c: median(tones.map(tone => tone.c)) };
}

/** An ideology's content colour in the palette's tone: the hue the content (or a mod)
 * gives it, at the palette's lightness and no more than its chroma, so the bands sit
 * with the app's other categories. A grey stays grey. */
export function ideologyColour([r, g, b]: readonly [number, number, number], tone: { l: number; c: number } | null): string {
  const source = rgbToOklch({ r, g, b });
  if (!tone) return toHex({ r, g, b });
  return toHex(oklchToRgb({ l: tone.l, c: source.c < GREY_CHROMA ? source.c : Math.min(source.c, tone.c), h: source.h }));
}

/** The ruling party's band first (the bottom of the stack and the legend's first
 * entry), the others in content order. */
export function rulingFirst(list: readonly Band[], ruling: string | null): Band[] {
  const index = list.findIndex(band => band.token === ruling);
  return index <= 0 ? [...list] : [list[index], ...list.slice(0, index), ...list.slice(index + 1)];
}

/** Every band's lines, in band order. */
export const bandKeys = (list: readonly Band[]): SeriesKey[] => list.flatMap(band => band.keys);

/** A band's exact popularity at an hour as read (`reached`: the states the widget read, by key): undefined while
 * the hour is not read, null when it was not recorded. */
export function rawAt(band: Band, reached: readonly ReachedKeyLine[], hour: number): string | null | undefined {
  let read = false;
  for (const line of reached) {
    if (!band.keys.some(key => key.tag === line.key.tag && key.part === line.key.part)) continue;
    const run = runAt(line.runs, hour);
    if (run) return run.valueRaw;
    read ||= line.missing.has(hour);
  }
  return read ? null : undefined;
}

export const formatPopularity = (raw: string, decimals: number) => `${formatFixed(raw, decimals, 1)}%`;

/** A popularity as a number (a followed value at the head), as `formatPopularity` shows an exact one. */
export const formatShare = (value: number) => `${value.toFixed(1)}%`;

/** A band's line as its tiles draw it (`summaryOf`, by key id): its one part's, or its parts' joined where its party
 * was read under several; undefined while none has landed. */
export function bandSummary(band: Band, summaryOf: (id: string) => ChartSummary | undefined): ChartSummary | undefined {
  const held = band.keys.flatMap(key => { const summary = summaryOf(keyId(key)); return summary ? [summary] : []; });
  if (held.length < 2) return held[0];
  try { return mergeSummaries(held); } catch { return held[0]; }
}

/** `bandSummary` as a chart draws it each frame: the same summary while its keys' summaries are the same objects, so
 * a frame merges nothing and the stack built from it is kept. */
export function bandSummaries(): (band: Band, summaryOf: (id: string) => ChartSummary | undefined) => ChartSummary | undefined {
  const made = new Map<string, { parts: (ChartSummary | undefined)[]; summary: ChartSummary | undefined }>();
  return (band, summaryOf) => {
    const parts = band.keys.map(key => summaryOf(keyId(key)));
    const held = made.get(band.token);
    if (held && held.parts.length === parts.length && held.parts.every((part, at) => part === parts[at])) return held.summary;
    const summary = bandSummary(band, summaryOf);
    made.set(band.token, { parts, summary });
    return summary;
  };
}

/** The ruling party's token at an hour, or null when not recorded. */
export function rulingAt(reply: PartyHistoryReply, hour: number): string | null {
  return reply.ruling.find(run => run.firstHour <= hour && hour <= run.lastHour)?.token ?? null;
}

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

/** The hours at which the government, its leader or the party's name can change, or the parties go unread: between
 * two of them the header shows the same, so a widget following the head renders only at them. */
export function partyChangeHours(reply: Pick<PartyHistoryReply, 'ruling' | 'leaders' | 'partyNameRuns' | 'unobserved'>): number[] {
  const hours = new Set<number>();
  for (const run of [...reply.ruling, ...reply.leaders, ...reply.partyNameRuns, ...reply.unobserved]) { hours.add(run.firstHour); hours.add(run.lastHour + 1); }
  return [...hours].sort((a, b) => a - b);
}

/** The hour the keyboard steps to among `hours` (sorted) from `from` within `bounds`: the next after it, the one
 * before it, the first or the last; null when there is none. */
export function stepHour(hours: readonly number[], key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null,
  bounds: readonly [number, number]): number | null {
  const inside = hours.filter(hour => hour >= bounds[0] && hour <= bounds[1]);
  switch (key) {
    case 'Home': return inside[0] ?? null;
    case 'End': return inside.at(-1) ?? null;
    case 'ArrowRight': return inside.find(hour => from === null || hour > from) ?? null;
    case 'ArrowLeft': return [...inside].reverse().find(hour => from === null || hour < from) ?? null;
  }
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
  const ideology = contentTitle(reply.ideologyTypes.find(item => item.token === run.ideology)?.name,
    run.ideology, 'Ideology name unavailable');
  const person = run.person == null ? undefined : reply.people[run.person];
  if (!person) return none('Leader not identified', ideology);
  return { name: person.name ?? 'Leader’s name not known', known: person.name != null, portrait: person.portrait, portraitMissing: person.portraitMissing, ideology };
}
