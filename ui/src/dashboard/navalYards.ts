import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from './keyedHistory.ts';
import type { KeyedWindow } from './keyedHistory';

/** Naval repairs and refits (`naval-repairs`, `naval-refits`): one country's keyed window,
 * the groups (ports or refit lines) it catalogues and the names of its interned tokens. */
export interface YardChoice { id: string; name: string }
export interface YardGroup { country: string; id: string; name: string; ships: readonly YardChoice[] }
export interface YardsReply { window: KeyedWindow; groups: readonly YardGroup[]; tokens: Readonly<Record<string, YardChoice>>; warnings: readonly string[] }

/** Fixed-point operands are 100,000 per unit. */
export const FIXED = 100000n;

const isChoice = (value: unknown): value is YardChoice => !!value && typeof value === 'object'
  && typeof (value as YardChoice).id === 'string' && typeof (value as YardChoice).name === 'string';

/** Validate a repairs (`ports`) or refits (`lines`) reply into one shape. */
export function validateYards(value: unknown, field: 'ports' | 'lines'): YardsReply {
  const reply = value as Record<string, unknown> & { window: unknown; tokens: unknown; warnings: unknown };
  const groups = reply?.[field] as YardGroup[] | undefined;
  if (!reply || typeof reply !== 'object' || !Array.isArray(groups) || !groups.every(group => isChoice(group) && typeof group.country === 'string'
      && Array.isArray(group.ships) && group.ships.every(isChoice))
    || !reply.tokens || typeof reply.tokens !== 'object' || !Object.values(reply.tokens as object).every(isChoice)
    || !Array.isArray(reply.warnings) || !(reply.warnings as unknown[]).every(warning => typeof warning === 'string'))
    throw new Error('The naval yards reply is malformed.');
  // The root keeps the prepared prefix fields (`coverage`, `pending`) the dashboard marks on it.
  return { ...reply, window: validateKeyedWindow(reply.window), groups, tokens: reply.tokens as Record<string, YardChoice>, warnings: reply.warnings as string[] };
}

/** One group's measures and ships at an hour, with its queue (ship handles in recorded order). */
export interface YardShip { id: string; measures: Map<string, bigint> }
export interface YardGroupAt { id: string; measures: Map<string, bigint>; ships: Map<string, YardShip>; queue: { ordinal: number; ship: string }[] }
export interface YardsAt { groups: Map<string, YardGroupAt>; totals: Map<string, bigint>; known: Map<string, bigint>; readHour: number | undefined }

/** The country's groups at `hour`; null when not recorded then. */
export function yardsAt(reply: YardsReply, tag: string, hour: number): YardsAt | null {
  if (!keyedKnown(reply.window, hour)) return null;
  const at: YardsAt = { groups: new Map(), totals: new Map(), known: new Map(), readHour: keyedRead(reply.window, hour) };
  for (const series of reply.window.series) {
    const parts = series.name.split('|');
    if (countryPart(parts[0])?.tag !== tag) continue;
    const run = runAtHour(series.runs, hour);
    if (!run) continue;
    const value = BigInt(run[2]);
    const [, group, kind, ...rest] = parts;
    if (group === 'total') { at.totals.set(kind, value); continue; }
    if (group === 'known' || group === 'read') { at.known.set(`${group}|${kind}`, value); continue; }
    if (group === 'refusal') continue;
    const entry: YardGroupAt = at.groups.get(group) ?? { id: group, measures: new Map(), ships: new Map(), queue: [] };
    if (kind === 'ship' && rest.length >= 2) {
      const ship = entry.ships.get(rest[0]) ?? { id: rest[0], measures: new Map() };
      ship.measures.set(rest.slice(1).join('|'), value);
      entry.ships.set(rest[0], ship);
    } else if (kind === 'queue' && rest.length === 1) entry.queue.push({ ordinal: Number(rest[0]), ship: value.toString() });
    else if (rest.length === 0) entry.measures.set(kind, value);
    at.groups.set(group, entry);
  }
  for (const group of at.groups.values()) group.queue.sort((a, b) => a.ordinal - b.ordinal);
  return at;
}

/** An interned token's name, or the fallback when the token or its name is not recorded. */
export function tokenName(reply: YardsReply, id: bigint | undefined, fallback: string): string {
  return id === undefined ? fallback : reply.tokens[id.toString()]?.name ?? fallback;
}

/** A ship's hull as a share of its maximum, when both are recorded and the maximum is positive. */
export function hullShare(ship: YardShip | undefined): number | null {
  const hull = ship?.measures.get('hull'), maximum = ship?.measures.get('maximum');
  if (hull === undefined || maximum === undefined || maximum <= 0n) return null;
  return Number(hull * 10000n / maximum) / 10000;
}

/** A fixed-point amount as text. */
export function fixedText(value: bigint | undefined): string {
  if (value === undefined) return '—';
  const negative = value < 0n, magnitude = negative ? -value : value;
  const fraction = (magnitude % FIXED).toString().padStart(5, '0').slice(0, 2).replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / FIXED).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}

/** A count the backend records unscaled (dockyards, ships), as an integer. */
export const countText = (value: bigint | undefined) => value === undefined ? '—' : value.toLocaleString('en-US');

/** Whether a fixed-point word is a value rather than one of the engine's sentinels (the
 * largest word and the two smallest), as the backend's own finite check reads them. */
export const finiteFixed = (raw: bigint) => raw !== 9223372036854775807n && raw !== -9223372036854775808n && raw !== -9223372036854775807n;

/** A native fixed-point IC word (refit cost or progress) as text: a sentinel is not a value. */
export const nativeIcText = (value: bigint | undefined) => value === undefined || !finiteFixed(value) ? 'Not recorded' : `${fixedText(value)} IC`;

/** A derived IC amount (the remaining cost the backend computes in wider arithmetic from
 * admitted operands) as text: any value it records is real, whatever its width. */
export const derivedIcText = (value: bigint | undefined) => value === undefined ? 'Not recorded' : `${fixedText(value)} IC`;

/** The supplying read that dates a refit line's daily cost, or undefined when none is recorded. */
export const costRead = (at: YardsAt): number | undefined => {
  const read = at.known.get('read|ProductionLines');
  return read === undefined ? undefined : Number(read);
};

/** A refit line's progress as a share of its cost: accumulated IC (hourly) over the positive
 * cost of its dated daily read; null when either is a sentinel or missing, the cost is not
 * positive, or the cost has no supplying read. */
export function refitShare(at: YardsAt, line: string): number | null {
  const group = at.groups.get(line);
  const progress = group?.measures.get('progress'), cost = group?.measures.get('cost');
  if (progress === undefined || cost === undefined || !finiteFixed(progress) || !finiteFixed(cost) || cost <= 0n) return null;
  if (costRead(at) === undefined) return null;
  return Number(progress * 1000000n / cost) / 1000000;
}

/** Whether the census behind an empty list is complete at the hour (a total the backend writes
 * as 1 only when every owner, link and row of that inventory was read). */
export const censusComplete = (at: YardsAt, total: string) => at.totals.get(total) === 1n;

export const percentText = (share: number | null) => share === null ? '—' : `${(share * 100).toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
