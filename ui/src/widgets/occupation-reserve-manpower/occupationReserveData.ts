import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { Run } from '../../charts/series';
import type { TooltipContent } from '../../charts/tooltip';
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { Country, OccupationReserveReply, ReserveHistoryReply, ReserveHolder, ReserveSegment } from '../../generated/dashboard';


const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const country = (value: unknown) => object(value) && Number.isSafeInteger(value.id) && (value.tag === null || typeof value.tag === 'string' && value.tag.length > 0);
const integer = (value: unknown) => Number.isSafeInteger(value);
const warnings = (value: Record<string, unknown>) => Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string');

export function validateOccupationReserve(value: unknown): OccupationReserveReply {
  const ok = object(value) && (value.missing === null || typeof value.missing === 'string') && (value.readHour === null || integer(value.readHour)) && warnings(value)
    && Array.isArray(value.holders) && value.holders.every(holder => object(holder) && country(holder.country) && typeof holder.admitted === 'boolean'
      && Array.isArray(holder.entries) && holder.entries.every(entry => object(entry) && integer(entry.ordinal) && country(entry.source)));
  if (!ok) throw new Error('The occupation reserve reply is malformed.');
  return value as unknown as OccupationReserveReply;
}

export function validateReserveHistory(value: unknown): ReserveHistoryReply {
  const ok = object(value) && warnings(value) && Array.isArray(value.segments) && value.segments.every((segment, index, all) => object(segment)
      && integer(segment.firstHour) && integer(segment.lastHour) && (segment.firstHour as number) <= (segment.lastHour as number)
      && (index === 0 || (all[index - 1] as ReserveSegment).lastHour < (segment.firstHour as number)) && typeof segment.admitted === 'boolean'
      && (segment.entries === null ? typeof segment.unknown === 'string' : segment.unknown === null && Array.isArray(segment.entries)
        && segment.entries.every(entry => object(entry) && integer(entry.source) && integer(entry.amount))))
    && Array.isArray(value.sources) && value.sources.every(source => object(source) && integer(source.id) && Array.isArray(source.tags)
      && source.tags.every(tag => object(tag) && (tag.tag === null || typeof tag.tag === 'string') && integer(tag.firstHour) && integer(tag.lastHour)));
  if (!ok) throw new Error('The occupation reserve history is malformed.');
  return value as unknown as ReserveHistoryReply;
}

/** What renders the race: the holders and their entries' sources, never their amounts nor the read hour. */
export const reserveStructure = (reply: OccupationReserveReply) => JSON.stringify([reply.missing, reply.holders, reply.warnings]);

export const UNRESOLVED = 'unresolved';
/** A source's part id: its tag, or the reason it has none. */
export const sourceKey = (source: Country) => source.tag ?? UNRESOLVED;

/** One entry at the values read now: its amount undefined where not read. */
export interface ReadEntry { ordinal: number; source: Country; amount: number | undefined }
/** A holder's reserve at the values read now: its positive entries' sum (null while any is not read), its negative ones. */
export interface HolderReserve { holder: ReserveHolder; tag: string; entries: ReadEntry[]; positive: number | null; negative: ReadEntry[] }

export function reserveOf(holder: ReserveHolder, values: LiveValues<number>): HolderReserve {
  const entries = holder.entries.map(entry => ({ ...entry, amount: values.get(liveKey(`${holder.country.id}:${entry.ordinal}`, 'amount')) }));
  const unread = entries.some(entry => entry.amount === undefined);
  return { holder, tag: holder.country.tag ?? `#${holder.country.id}`, entries,
    positive: unread ? null : entries.reduce((sum, entry) => sum + Math.max(0, entry.amount!), 0),
    negative: entries.filter(entry => entry.amount !== undefined && entry.amount < 0) };
}

/** A reserve's positive entries summed by source (repeated sources add up on the bar; the tooltip keeps each entry),
 * largest first. */
export function sourceParts(reserve: HolderReserve): { source: string; amount: number }[] {
  const by = new Map<string, number>();
  for (const entry of reserve.entries) if (entry.amount !== undefined && entry.amount > 0) by.set(sourceKey(entry.source), (by.get(sourceKey(entry.source)) ?? 0) + entry.amount);
  return [...by].map(([source, amount]) => ({ source, amount })).sort((a, b) => b.amount - a.amount || a.source.localeCompare(b.source));
}

/** The holders the race may show: those a tag names, admitted or holding an entry, kept by `candidate`. A holder with no
 * entry holds a proven zero: a top N does not rank it, every country keeps it reachable, and when no holder has an entry
 * there is nothing to list, never a column of zeros in tag order. */
export function raceHolders(reply: OccupationReserveReply, candidate: (tag: string) => boolean, top = false): ReserveHolder[] {
  const held = reply.holders.filter(holder => holder.country.tag !== null && (holder.admitted || holder.entries.length > 0) && candidate(holder.country.tag));
  return held.some(holder => holder.entries.length > 0) ? top ? held.filter(holder => holder.entries.length > 0) : held : [];
}

/** The race at the values read now: most held first, ties by tag; the first `top` when given. A holder with an amount
 * not read is ranked last and drawn as nothing, never as zero. */
export function raceEntries(holders: readonly ReserveHolder[], values: LiveValues<number>, top: number | null,
  colour: (source: string) => string, hour: number): CountryRaceEntry[] {
  const reserves = holders.map(holder => reserveOf(holder, values)).filter(reserve => top === null || reserve.positive !== 0);
  reserves.sort((a, b) => (b.positive ?? -1) - (a.positive ?? -1) || a.tag.localeCompare(b.tag));
  const shown = top === null ? reserves : reserves.slice(0, Math.max(0, top));
  return shown.map(reserve => ({ tag: reserve.tag, hour, value: reserve.positive ?? 0, label: reserve.positive === null ? 'Not read' : persons(reserve.positive), stretch: null,
    parts: sourceParts(reserve).map(part => ({ id: part.source, value: part.amount, color: colour(part.source) })) }));
}

export const persons = (value: number) => value.toLocaleString('en-US');

export function holderTooltip(reserve: HolderReserve, name: string, sourceName: (source: Country) => string, sourceColour: (source: string) => string,
  read: string | null): TooltipContent {
  const positive = reserve.entries.filter(entry => entry.amount === undefined || entry.amount >= 0);
  return {
    title: name, scrollable: true, scrollId: String(reserve.holder.country.id),
    rows: [
      { label: 'Reserve manpower held', value: reserve.positive === null ? 'Not read' : persons(reserve.positive) },
      ...positive.map(entry => ({ label: sourceName(entry.source), swatch: sourceColour(sourceKey(entry.source)), value: entry.amount === undefined ? 'Not read' : persons(entry.amount), indent: true })),
      ...reserve.negative.length ? [{ label: 'Negative entries' }, ...reserve.negative.map(entry => ({ label: sourceName(entry.source), value: persons(entry.amount!), indent: true }))] : [],
      ...read ? [{ label: 'As of', value: read }] : [],
    ],
  };
}

/** A source's tag at `hour` in a history: the tag its raw id held then, else none. */
export function sourceTagAt(history: ReserveHistoryReply, source: number, hour: number): string | null {
  const tags = history.sources.find(item => item.id === source)?.tags ?? [];
  return tags.find(tag => tag.firstHour <= hour && hour <= tag.lastHour)?.tag ?? null;
}

/** The segment holding `hour`, if the history holds it. */
export const segmentAt = (history: ReserveHistoryReply, hour: number): ReserveSegment | null =>
  history.segments.find(segment => segment.firstHour <= hour && hour <= segment.lastHour) ?? null;

/** Each source's positive amount in a segment, by raw id; an admitted holder holds 0 from a source it has no entry from. */
export function segmentParts(segment: ReserveSegment, sources: readonly number[]): (number | undefined)[] {
  if (!segment.entries) return sources.map(() => undefined);
  const by = new Map<number, number>();
  for (const entry of segment.entries) if (entry.amount > 0) by.set(entry.source, (by.get(entry.source) ?? 0) + entry.amount);
  return sources.map(source => by.get(source) ?? (segment.admitted ? 0 : undefined));
}

/** The holder drill's bands: each source's positive reserve over the history, in `sources`' order, a gap where it was
 * not known; runs of equal value join. */
export function holderBands(history: ReserveHistoryReply, sources: readonly number[]): Run[][] {
  const bands = sources.map(() => [] as Run[]);
  for (const segment of history.segments) {
    segmentParts(segment, sources).forEach((value, band) => {
      if (value === undefined) return;
      const runs = bands[band], previous = runs.at(-1);
      if (previous && previous.lastHour + 1 === segment.firstHour && previous.value === value) previous.lastHour = segment.lastHour;
      else runs.push({ firstHour: segment.firstHour, lastHour: segment.lastHour, value, breakBefore: !previous || previous.lastHour + 1 !== segment.firstHour });
    });
  }
  return bands;
}

/** The sources a history names, those holding the most at any hour first. */
export function historySources(history: ReserveHistoryReply): number[] {
  const most = new Map<number, number>();
  for (const segment of history.segments) for (const entry of segment.entries ?? []) most.set(entry.source, Math.max(most.get(entry.source) ?? 0, entry.amount));
  return [...most].sort((a, b) => b[1] - a[1] || a[0] - b[0]).map(([source]) => source);
}

/** A pinned holder: its tag and raw id (`TAG|id`). */
export const holderToken = (holder: { tag: string; id: number }) => `${holder.tag}|${holder.id}`;
export function holderOf(token: unknown): { tag: string; id: number } | null {
  if (typeof token !== 'string') return null;
  const [tag, id] = token.split('|');
  const raw = Number(id);
  return tag && id !== undefined && Number.isSafeInteger(raw) ? { tag, id: raw } : null;
}
