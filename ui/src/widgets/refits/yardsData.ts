import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { NavalRefitsReply, NavalRepairsReply, RefitLine, RepairPort, YardShip } from '../../generated/dashboard';

/** A `naval-refits` reply: `missing` says why the refit lines are not known at the hour, when they are not. */
export type RefitsReply = NavalRefitsReply & { missing: string | null };

/** What refits not known at the hour show: plainly that, the reply's reason (which names its tables) in the tooltip. */
export const refitsMissing = (reply: RefitsReply): { message: string; reason: string } | null =>
  reply.missing === null ? null : { message: 'Refits were not recorded at this date.', reason: reply.missing };

/** Fixed-point operands are 100,000 per unit. */
export const FIXED = 100000n;

const text = (value: unknown): value is string => typeof value === 'string';
const optional = (value: unknown) => value === null || text(value);
const decimal = (value: unknown) => text(value) && /^-?\d+$/u.test(value);
const optionalDecimal = (value: unknown) => value === null || decimal(value);
const optionalInteger = (value: unknown) => value === null || Number.isSafeInteger(value);
const strings = (value: unknown) => Array.isArray(value) && value.every(text);
const ship = (value: unknown): value is YardShip => {
  const item = value as YardShip;
  return !!item && typeof item === 'object' && decimal(item.handle) && optional(item.name) && optional(item.nameRebuilt) && optional(item.design)
    && optional(item.class) && optionalDecimal(item.hull) && optionalDecimal(item.maximum) && optional(item.taskForce) && optional(item.policy);
};

export function validateRepairs(value: unknown): NavalRepairsReply {
  const reply = value as NavalRepairsReply;
  if (!reply || typeof reply !== 'object' || typeof reply.censusComplete !== 'boolean' || !strings(reply.warnings)
    || !optionalInteger(reply.usedDockyards) || !optionalInteger(reply.repairLimit) || !optionalInteger(reply.queuedShips)
    || !Array.isArray(reply.ports) || !reply.ports.every((port: RepairPort) => !!port && Number.isSafeInteger(port.province) && optional(port.name)
      && optionalInteger(port.priority) && optional(port.policy) && optionalInteger(port.dockyards) && Array.isArray(port.queue) && port.queue.every(ship)))
    throw new Error('The naval repairs reply is malformed.');
  return reply;
}

export function validateRefits(value: unknown): RefitsReply {
  const reply = value as RefitsReply;
  if (!reply || typeof reply !== 'object' || typeof reply.censusComplete !== 'boolean' || !strings(reply.warnings) || !optional(reply.missing)
    || !optionalInteger(reply.ships) || !optionalInteger(reply.dockyards) || !optionalInteger(reply.costRead)
    || !Array.isArray(reply.lines) || !reply.lines.every((line: RefitLine) => !!line && decimal(line.handle) && optional(line.target) && decimal(line.progress)
      && optionalDecimal(line.cost) && optionalInteger(line.dockyards) && optionalDecimal(line.remaining) && Array.isArray(line.ships) && line.ships.every(ship)))
    throw new Error('The naval refits reply is malformed.');
  return reply;
}

/** Where a ship's shown name comes from: the game's held name, one rebuilt from its recorded naming through the
 * content's name lists, or none (its design, else its class, then stands in). */
export type ShipNameSource = 'recorded' | 'rebuilt' | 'design';

export function shipLabel(item: YardShip | undefined): { name: string; source: ShipNameSource } {
  if (!item) return { name: 'Ship name unavailable', source: 'design' };
  if (item.name) return { name: item.name, source: 'recorded' };
  if (item.nameRebuilt) return { name: item.nameRebuilt, source: 'rebuilt' };
  return { name: item.design ?? item.class ?? 'Ship name unavailable', source: 'design' };
}

/** The tooltip row naming the shown name's source. */
export const shipNameRow = (source: ShipNameSource) => ({ label: 'Name', value: source === 'recorded' ? 'As the game held it; not checked against a save'
  : source === 'rebuilt' ? 'Rebuilt from the recorded naming; not checked against a save' : 'Not recorded; design shown' });

/** A ship's hull now: its live cell's, else the reply's. */
const liveHull = (item: YardShip, values: LiveValues<number>): YardShip => {
  const hull = values.get(liveKey(item.handle, 'hull'));
  return hull === undefined ? item : { ...item, hull: String(Math.trunc(hull)) };
};

/** A ship's hull as a share of its maximum, when both are recorded and the maximum is positive. */
export function hullShare(item: YardShip | undefined): number | null {
  if (!item?.hull || !item.maximum) return null;
  const maximum = BigInt(item.maximum);
  if (maximum <= 0n) return null;
  return Number(BigInt(item.hull) * 10000n / maximum) / 10000;
}

/** A fixed-point amount as text. */
export function fixedText(value: string | null | undefined): string {
  if (value === null || value === undefined) return '—';
  const raw = BigInt(value), negative = raw < 0n, magnitude = negative ? -raw : raw;
  const fraction = (magnitude % FIXED).toString().padStart(5, '0').slice(0, 2).replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / FIXED).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}

export const countText = (value: number | null | undefined) => value === null || value === undefined ? '—' : value.toLocaleString('en-US');
export const percentText = (share: number | null) => share === null ? '—' : `${(share * 100).toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
export const icText = (value: string | null) => value === null ? 'Not recorded' : `${fixedText(value)} IC`;

/** A refit line's progress as a share of its cost: null where the cost is unknown, not positive or its read unknown. */
export function refitShare(reply: NavalRefitsReply, line: RefitLine): number | null {
  if (line.cost === null || reply.costRead === null) return null;
  const cost = BigInt(line.cost);
  if (cost <= 0n || cost >= 9223372036854775807n) return null;
  return Number(BigInt(line.progress) * 1000000n / cost) / 1000000;
}

const named = (item: YardShip) => [item.handle, item.name, item.nameRebuilt, item.design, item.class, item.taskForce, item.policy, item.maximum];

/** What a repairs render shows: ports, queues and names; hulls move hourly and are painted from their live cells. */
export function repairsStructure(reply: NavalRepairsReply): string {
  return JSON.stringify([reply.ports.map(port => [port.province, port.name, port.priority, port.policy, port.dockyards, port.queue.map(named)]),
    reply.usedDockyards, reply.repairLimit, reply.queuedShips, reply.censusComplete]);
}

/** The hull texts painted in place, by `data-paint` key (port and place in queue), from the live hulls. */
export function repairsTexts(reply: NavalRepairsReply, values: LiveValues<number>): Map<string, string> {
  return new Map(reply.ports.flatMap(port => port.queue.map((item, index) => [`hull|${port.province}|${index}`, percentText(hullShare(liveHull(item, values)))] as const)));
}

/** What a refits render shows: lines, ships and names; progress and hulls move hourly and are painted. */
export function refitsStructure(reply: RefitsReply): string {
  return JSON.stringify([reply.lines.map(line => [line.handle, line.target, line.cost, line.dockyards, line.ships.map(named)]),
    reply.ships, reply.dockyards, reply.censusComplete, reply.costRead, reply.missing]);
}

/** A line's progress now: its live cell's, else the reply's. */
const liveLine = (line: RefitLine, values: LiveValues<number>): RefitLine => {
  const progress = values.get(liveKey(line.handle, 'progress'));
  return progress === undefined ? line : { ...line, progress: String(Math.trunc(progress)) };
};

export function refitsTexts(reply: NavalRefitsReply, values: LiveValues<number>): Map<string, string> {
  return new Map(reply.lines.flatMap(line => [[`progress|${line.handle}`, percentText(refitShare(reply, liveLine(line, values)))] as const,
    ...line.ships.map(item => [`hull|${line.handle}|${item.handle}`, percentText(hullShare(liveHull(item, values)))] as const)]));
}
