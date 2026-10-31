import type { NavalYardsAtReply } from '../generated/dashboard';

/** Naval repairs and refits (`naval-repairs`, `naval-refits`): one country's ports or refit
 * lines at an hour, their ships, its totals and the names of the tokens they name. */
export type YardsReply = NavalYardsAtReply;
export interface YardChoice { id: string; name: string }

/** Fixed-point operands are 100,000 per unit. */
export const FIXED = 100000n;

const isChoice = (value: unknown): value is YardChoice => !!value && typeof value === 'object'
  && typeof (value as YardChoice).id === 'string' && typeof (value as YardChoice).name === 'string';
const decimals = (value: unknown): value is Record<string, string> => !!value && typeof value === 'object'
  && Object.values(value as object).every(item => typeof item === 'string' && /^-?\d+$/.test(item));
const decimal = (value: unknown) => typeof value === 'string' && /^-?\d+$/.test(value);
const isShip = (value: unknown) => !!value && typeof value === 'object'
  && typeof (value as { id: unknown }).id === 'string' && decimals((value as { measures: unknown }).measures);

/** Validate a repairs or refits reply at an hour. */
export function validateYards(value: unknown): YardsReply {
  const reply = value as Partial<YardsReply> | null;
  if (!reply || typeof reply !== 'object' || typeof reply.hour !== 'number' || typeof reply.known !== 'boolean'
    || !(reply.readHour === null || typeof reply.readHour === 'number')
    || !Array.isArray(reply.holders) || !reply.holders.every(holder => isShip(holder) && Array.isArray(holder.ships) && holder.ships.every(isShip)
      && Array.isArray(holder.queue) && holder.queue.every(entry => !!entry && decimal(entry.ordinal) && decimal(entry.ship)))
    || !decimals(reply.totals) || !reply.reads || typeof reply.reads !== 'object' || !Object.values(reply.reads).every(Number.isInteger)
    || !reply.tokens || typeof reply.tokens !== 'object' || !Object.values(reply.tokens).every(isChoice)
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The naval yards reply is malformed.');
  return reply as YardsReply;
}

/** One group's measures and ships at an hour, with its queue (ship handles in recorded order). */
export interface YardShip { id: string; measures: Map<string, bigint> }
export interface YardGroupAt { id: string; measures: Map<string, bigint>; ships: Map<string, YardShip>; queue: { ordinal: number; ship: string }[] }
export interface YardsAt { groups: Map<string, YardGroupAt>; totals: Map<string, bigint>; known: Map<string, bigint>; readHour: number | undefined }

const measured = (measures: Readonly<Partial<Record<string, string>>>) => new Map(Object.entries(measures)
  .flatMap(([name, value]) => value === undefined ? [] : [[name, BigInt(value)] as const]));

/** The reply's groups at its hour; null where the hour's countries were not recorded. */
export function yardsAt(reply: YardsReply): YardsAt | null {
  if (!reply.known) return null;
  const groups = new Map(reply.holders.map(holder => [holder.id, {
    id: holder.id, measures: measured(holder.measures),
    ships: new Map(holder.ships.map(ship => [ship.id, { id: ship.id, measures: measured(ship.measures) }] as const)),
    queue: holder.queue.map(entry => ({ ordinal: Number(entry.ordinal), ship: entry.ship })),
  }] as const));
  const known = new Map(Object.entries(reply.reads)
    .flatMap(([table, hour]) => hour === undefined ? [] : [[`read|${table}`, BigInt(hour)] as const]));
  return { groups, totals: measured(reply.totals), known, readHour: reply.readHour ?? undefined };
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
