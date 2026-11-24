import { liveKey } from '../../charts/live.ts';
import { NOT_LIVE_CELL } from '../../dashboard/follow/liveCells.ts';
import type { LiveValues } from '../../charts/live';
import { knownValue, notRecorded, partialValue, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import type { TableCellExtra, TooltipExtraRow } from '../../families/table/model.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { cellArt, type CellArt } from '../../families/table/identityCell.ts';
import type { NavalRefitsReply, NavalRepairsReply, RefitLine, RepairPort, YardShip } from '../../generated/dashboard';

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
      && optionalInteger(port.priority) && optional(port.policy) && optionalInteger(port.dockyards) && Array.isArray(port.queue) && port.queue.every(ship) && optional(port.queueMissing)))
    throw new Error('The naval repairs reply is malformed.');
  return reply;
}

export function validateRefits(value: unknown): NavalRefitsReply {
  const reply = value as NavalRefitsReply;
  if (!reply || typeof reply !== 'object' || typeof reply.censusComplete !== 'boolean' || !strings(reply.warnings) || !optional(reply.missing)
    || !optionalInteger(reply.ships) || !optionalInteger(reply.dockyards) || !optionalInteger(reply.costRead)
    || !Array.isArray(reply.lines) || !reply.lines.every((line: RefitLine) => !!line && decimal(line.handle) && optional(line.target) && decimal(line.progress)
      && optionalDecimal(line.cost) && optionalInteger(line.dockyards) && optionalDecimal(line.remaining) && Array.isArray(line.ships) && line.ships.every(ship) && optional(line.shipsMissing)))
    throw new Error('The naval refits reply is malformed.');
  return reply;
}

export function shipPicture(ship: YardShip, readView: BoundedReadView): CellArt {
  return cellArt(readView, ship.classIcon ? { kind: 'sprite', key: ship.classIcon.key, frame: ship.classIcon.frame } : null, 'The recording names no counter for this ship class');
}

export function shipPictureCell(ship: YardShip, picture: CellArt): TableCellExtra {
  return { iconSlot: true, artShape: 'counter', ...(ship.class ? { artName: ship.class } : {}), ...picture };
}

export type ShipNameSource = 'recorded' | 'rebuilt' | 'design';

export function shipLabel(item: YardShip): { name: string; source: ShipNameSource } {
  if (item.name) return { name: item.name, source: 'recorded' };
  if (item.nameRebuilt) return { name: item.nameRebuilt, source: 'rebuilt' };
  return { name: item.design ?? item.class ?? 'Ship name unavailable', source: 'design' };
}

const NAME_SOURCE: Readonly<Record<ShipNameSource, string>> = {
  recorded: 'As the game held it',
  rebuilt: 'Rebuilt from the recorded naming',
  design: 'Not recorded; design shown',
};

export function fixedText(value: string | bigint | null | undefined): string {
  if (value === null || value === undefined) return 'Not recorded';
  const raw = BigInt(value), negative = raw < 0n, magnitude = negative ? -raw : raw;
  const fraction = (magnitude % FIXED).toString().padStart(5, '0').slice(0, 2).replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / FIXED).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}

function shareValue(part: bigint, whole: bigint): Value<number> {
  if (part <= 0n) return zeroValue(0);
  return knownValue(Number((part > whole ? whole : part) * 1_000_000n / whole) / 10_000);
}

type Live = LiveValues<number | bigint | string> | null;

export function reading(live: Live, row: string, column: string, held: string | null, reason: string): bigint | Value<never> {
  if (live) {
    const value = valueFromLive(live, liveKey(row, column));
    if (value.kind === 'known' || value.kind === 'zero') return typeof value.value === 'bigint' ? value.value : BigInt(Math.trunc(value.value));
    if (value.kind === 'notRecorded' && value.reason !== NOT_LIVE_CELL) return notRecorded(value.reason);
  }
  return held === null ? notRecorded(reason) : BigInt(held);
}

const hullReading = (item: YardShip, live: Live) => reading(live, item.handle, 'hull', item.hull, 'Ship strength not recorded');

export function shipStrength(item: YardShip, live: Live): Value<number> {
  const maximum = item.maximum === null ? 0n : BigInt(item.maximum);
  if (maximum <= 0n) return notRecorded('Maximum ship strength not recorded');
  const hull = hullReading(item, live);
  return typeof hull === 'bigint' ? shareValue(hull, maximum) : hull;
}

const LONGEST = 9223372036854775807n;

function refitCost(reply: NavalRefitsReply, line: RefitLine): bigint | null {
  const cost = line.cost === null || reply.costRead === null ? 0n : BigInt(line.cost);
  return cost <= 0n || cost >= LONGEST ? null : cost;
}

export const progressReading = (line: RefitLine, live: Live) => reading(live, line.handle, 'progress', line.progress, 'Refit progress not recorded');

export function refitProgress(reply: NavalRefitsReply, line: RefitLine, live: Live): Value<number> {
  const cost = refitCost(reply, line);
  if (cost === null) return notRecorded('Refit cost not recorded');
  const progress = progressReading(line, live);
  return typeof progress === 'bigint' ? shareValue(progress, cost) : progress;
}

const icValue = (raw: bigint | Value<never>): Value<string> => typeof raw === 'bigint' ? knownValue(`${fixedText(raw)} IC`) : raw;

export function refitAmounts(reply: NavalRefitsReply, line: RefitLine, live: Live): { readonly done: Value<string>; readonly cost: Value<string>; readonly remaining: Value<string> } {
  const progress = progressReading(line, live);
  const cost = refitCost(reply, line);
  const remaining = cost === null ? notRecorded('Remaining cost not recorded') : typeof progress === 'bigint' ? icValue(cost - progress) : progress;
  return { done: icValue(progress), cost: cost === null ? notRecorded('Refit cost not recorded') : icValue(cost), remaining };
}

export function childCount(count: number, missing: string | null): Value<string> {
  if (missing === null) return knownValue(formatScalar(count, 'integer'));
  return count > 0 ? partialValue(formatScalar(count, 'integer'), missing) : notRecorded(missing);
}

export const fraction = (value: Value<number>): Value<number> =>
  value.kind === 'known' ? knownValue(value.value / 100) : value;

export function shipTooltip(item: YardShip, live: Live): TooltipExtraRow[] {
  const label = shipLabel(item);
  const named = (value: string | null, reason: string): Value<string> => value ? knownValue(value) : notRecorded(reason);
  const hull = hullReading(item, live);
  return [
    { label: 'Name', value: knownValue(NAME_SOURCE[label.source]), divider: true },
    { label: 'Strength', value: typeof hull !== 'bigint' ? hull : item.maximum === null ? notRecorded('Maximum ship strength not recorded') : knownValue(`${fixedText(hull)} of ${fixedText(item.maximum)}`) },
    { label: 'Design', value: named(item.design, 'Design name unavailable') },
    { label: 'Class', value: named(item.class, 'Ship class not recorded') },
    { label: 'Task force', value: named(item.taskForce, 'Task force not recorded') },
    { label: 'Repair policy', value: named(item.policy, 'Repair policy not recorded') },
  ];
}
