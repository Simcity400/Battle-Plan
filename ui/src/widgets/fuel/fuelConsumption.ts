import { object } from '../../dashboard/validation.ts';
import { isRaw } from './fuelData.ts';
import type { FuelBasis } from './fuelDrill';
import type { FuelDemandHistoryReply, FuelDemandPart, FuelDemandReply } from '../../generated/dashboard';

const text = (value: unknown) => value === null || typeof value === 'string';
const int = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
const raw = (value: unknown) => value === null || isRaw(value);
const strings = (value: unknown) => Array.isArray(value) && value.every(item => typeof item === 'string');
const triple = (value: unknown, each: (item: unknown) => boolean) => Array.isArray(value) && value.length === 3 && value.every(each);

export function validateFuelDemand(value: unknown): FuelDemandReply {
  const reply = value as FuelDemandReply;
  const consumption = object(value) ? value.consumption : null;
  if (!object(value) || typeof reply.tag !== 'string' || !int(reply.readHour) || !int(reply.country) || !(reply.countryInstance === null || int(reply.countryInstance))
    || !object(consumption) || !triple(consumption.nativeDailyRaw, raw) || !triple(consumption.knownDailyRaw, raw) || !triple(consumption.positiveDailyRaw, raw)
    || !triple(consumption.reasons, text) || !Array.isArray(reply.parts)
    || !reply.parts.every(part => object(part) && typeof part.key === 'string' && !!part.key && int(part.kind) && part.kind >= 0 && part.kind <= 2
      && typeof part.reference === 'string' && typeof part.parent === 'string' && int(part.ordinal) && text(part.group) && text(part.topologyReason)
      && raw(part.amountRaw) && text(part.reason) && (part.amountRaw === null) === (part.reason !== null) && text(part.recordedName) && text(part.nameReason))
    || new Set(reply.parts.map(part => part.key)).size !== reply.parts.length
    || !Array.isArray(reply.commands) || !reply.commands.every(group => object(group) && typeof group.reference === 'string' && typeof group.armyGroup === 'boolean'
      && strings(group.members) && strings(group.children) && text(group.recordedName))
    || !Array.isArray(reply.fleets) || !reply.fleets.every(fleet => object(fleet) && typeof fleet.reference === 'string' && text(fleet.recordedName))
    || !Array.isArray(reply.sites) || !reply.sites.every(site => object(site) && int(site.list) && int(site.ordinal) && typeof site.base === 'string'
      && raw(site.capacityRaw) && text(site.reason) && strings(site.wings))
    || !text(reply.topologyReason) || !strings(reply.warnings)) throw new Error('The individual fuel consumption detail is malformed.');
  return reply;
}

export function validateFuelDemandHistory(value: unknown): FuelDemandHistoryReply {
  const reply = value as FuelDemandHistoryReply;
  if (!object(value) || typeof reply.tag !== 'string' || !int(reply.country) || !int(reply.window) || !int(reply.firstHour) || !int(reply.lastHour)
    || !Array.isArray(reply.parts) || !strings(reply.warnings)
    || !reply.parts.every(part => object(part) && typeof part.key === 'string' && Array.isArray(part.runs)
      && part.runs.every((run, i) => object(run) && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour && typeof run.startsSegment === 'boolean'
        && raw(run.amountRaw) && text(run.reason) && (run.amountRaw === null) === (run.reason !== null) && (i === 0 || part.runs[i - 1].lastHour < run.firstHour))))
    throw new Error('The individual fuel consumption history is malformed.');
  return reply;
}

export const siteLevel = (site: FuelDemandReply['sites'][number], index: number) =>
  `site:${JSON.stringify([index, site.list, site.ordinal, site.base, site.containerOwner])}`;

export function consumptionParts(reply: FuelDemandReply, level: string): FuelDemandPart[] {
  if (level === 'all') return reply.parts;
  if (/^family:[012]$/.test(level)) return reply.parts.filter(part => part.kind === Number(level.slice(7)));
  const group = reply.commands.find(command => `command:${command.reference}` === level);
  if (group) {
    const commands = new Map(reply.commands.map(command => [command.reference, command]));
    const reached = new Set<string>(), members = new Set<string>(), pending = [group.reference];
    while (pending.length) {
      const reference = pending.pop()!;
      if (reached.has(reference)) continue;
      reached.add(reference);
      const command = commands.get(reference);
      if (command) { for (const member of command.members) members.add(member); for (const child of command.children) pending.push(child); }
    }
    return reply.parts.filter(part => part.kind === 0 && (part.group !== null && reached.has(part.group) || members.has(part.reference)));
  }
  const fleet = reply.fleets.find(item => `fleet:${item.reference}` === level);
  if (fleet) return reply.parts.filter(part => part.kind === 2 && part.parent === fleet.reference);
  const site = reply.sites.find((item, index) => siteLevel(item, index) === level);
  if (site) { const keys = new Set(site.wings); return reply.parts.filter(part => keys.has(part.key)); }
  return [];
}

export function consumptionLevelAvailable(reply: FuelDemandReply, level: string): boolean {
  return level === 'all' || /^family:[012]$/.test(level) || reply.commands.some(group => `command:${group.reference}` === level)
    || reply.fleets.some(fleet => `fleet:${fleet.reference}` === level) || reply.sites.some((site, index) => siteLevel(site, index) === level);
}

export function levelComplete(reply: FuelDemandReply, level: string, shown: readonly FuelDemandPart[]): boolean {
  if (shown.some(part => part.amountRaw === null)) return false;
  if (level.startsWith('site:')) {
    const site = reply.sites.find((item, index) => siteLevel(item, index) === level);
    return site?.reason === null && site.wings.length === shown.length;
  }
  const families = /^family:[012]$/.test(level) ? [Number(level.slice(7))] : level.startsWith('command:') ? [0] : level.startsWith('fleet:') ? [2] : [0, 1, 2];
  return families.every(kind => reply.consumption.reasons[kind] === null) && (!level.startsWith('command:') || reply.topologyReason === null);
}

export interface FuelConsumptionLevel { campaign: string; branch: string; basis: FuelBasis; level: string }
export interface FuelConsumptionSelection { campaign: string; branch: string; basis: FuelBasis; key: string; name: string | null }
export function demandBasis(branchId: string, reply: FuelDemandReply): FuelBasis | null {
  return reply.countryInstance === null ? null : { branchId, tag: reply.tag, hour: reply.readHour, country: reply.country, countryInstance: reply.countryInstance };
}

export function demandGeometry(runs: readonly { firstHour: number; lastHour: number; startsSegment: boolean; amountRaw: string | null }[], scale: number) {
  return runs.flatMap((run, i) => run.amountRaw === null ? [] : [{ firstHour: run.firstHour, lastHour: run.lastHour,
    value: Math.max(0, Number(run.amountRaw) / scale), breakBefore: run.startsSegment || i === 0 || runs[i - 1].amountRaw === null || runs[i - 1].lastHour + 1 !== run.firstHour }]);
}

export function demandHistoryRange(
  extent: readonly [number, number],
  anchorHour: number | null | undefined,
  detailHours: number,
): readonly [number, number] {
  const end = Math.max(extent[0], Math.min(extent[1], anchorHour ?? extent[1]));
  const start = Math.max(extent[0], end - detailHours + 1);
  return [start, end];
}
