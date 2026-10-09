import type { FuelAwardsParams, FuelConsumptionRead, FuelConsumptionReply, FuelDemandHistoryReply, FuelDemandPart, FuelDemandReply, FuelDemandRun, FuelDemandStackReply } from '../../generated/dashboard';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { object, signed32, unsigned32, unsigned64 } from '../../dashboard/validation.ts';
import { runAt } from '../../charts/series.ts';
import { clipToSpans } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { isRaw } from './fuelData.ts';

const text = (value: unknown) => value === null || typeof value === 'string';
const reference = (value: unknown) => unsigned64(value) && BigInt(value) < (1n << 32n);
const raw = (value: unknown) => value === null || isRaw(value);
const span = (value: Record<string, unknown>): value is Record<string, unknown> & Pick<FuelConsumptionRead, 'firstHour' | 'lastHour'> =>
  signed32(value.firstHour) && signed32(value.lastHour) && value.firstHour <= value.lastHour;
const strings = (value: unknown) => Array.isArray(value) && value.every(item => typeof item === 'string');
const refs = (value: unknown) => Array.isArray(value) && value.every(unsigned64);
const read = (value: unknown): value is FuelConsumptionRead => {
  if (!object(value)) return false;
  const reasons = value.reasons;
  return span(value) && typeof value.startsSegment === 'boolean'
  && signed32(value.segmentStartHour) && value.segmentStartHour <= value.firstHour && reference(value.country)
  && (value.countryInstance === null || unsigned32(value.countryInstance))
  && Array.isArray(value.nativeDailyRaw) && value.nativeDailyRaw.length === 3 && value.nativeDailyRaw.every(raw)
  && Array.isArray(value.knownDailyRaw) && value.knownDailyRaw.length === 3 && value.knownDailyRaw.every(isRaw)
  && Array.isArray(value.positiveDailyRaw) && value.positiveDailyRaw.length === 3 && value.positiveDailyRaw.every(amount => isRaw(amount) && BigInt(amount) >= 0n)
  && Array.isArray(reasons) && reasons.length === 3 && reasons.every(text)
  && value.nativeDailyRaw.every((amount, i) => (amount === null) === (reasons[i] !== null));
};

export function validateFuelConsumption(value: unknown): FuelConsumptionReply {
  const reply = value as FuelConsumptionReply;
  if (!object(value) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !Array.isArray(reply.reads)
    || !reply.reads.every(read) || !reply.reads.every((item, i) => i === 0 || reply.reads[i - 1].lastHour < item.firstHour) || !strings(reply.warnings)) {
    throw new Error('The individual fuel consumption history is malformed.');
  }
  return reply;
}
export function validateFuelDemand(value: unknown): FuelDemandReply {
  const reply = value as FuelDemandReply;
  if (!object(value) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !signed32(reply.hour)
    || !read(reply.read) || reply.read.firstHour !== reply.hour || !Array.isArray(reply.parts)
    || !reply.parts.every(part => object(part) && typeof part.key === 'string' && !!part.key && unsigned32(part.kind) && part.kind <= 2
      && unsigned64(part.reference) && unsigned64(part.parent) && unsigned32(part.ordinal)
      && (part.owners === null || Array.isArray(part.owners) && part.owners.length === 3 && part.owners.every(reference))
      && (part.wingOwner === null || reference(part.wingOwner)) && (part.group === null || unsigned64(part.group))
      && text(part.topologyReason) && raw(part.amountRaw) && text(part.reason) && (part.amountRaw === null) === (part.reason !== null)
      && text(part.recordedName) && text(part.nameReason)
      && (part.site === null || object(part.site) && unsigned32(part.site.list) && part.site.list <= 255 && unsigned32(part.site.siteOrdinal)
        && unsigned64(part.site.base) && (part.site.containerOwner === null || reference(part.site.containerOwner))
        && unsigned32(part.site.poolOrdinal) && unsigned32(part.site.wingOrdinal) && text(part.site.reason)))
    || new Set(reply.parts.map(part => part.key)).size !== reply.parts.length
    || !Array.isArray(reply.commands) || !reply.commands.every(group => object(group) && unsigned64(group.reference) && unsigned64(group.parent)
      && reference(group.country) && typeof group.armyGroup === 'boolean' && refs(group.members) && refs(group.children) && text(group.recordedName) && text(group.nameReason))
    || !Array.isArray(reply.fleets) || !reply.fleets.every(fleet => object(fleet) && unsigned64(fleet.reference) && text(fleet.recordedName) && text(fleet.nameReason))
    || !Array.isArray(reply.sites) || !reply.sites.every(site => object(site) && unsigned32(site.list) && site.list <= 255 && unsigned32(site.ordinal)
      && unsigned64(site.base) && (site.containerOwner === null || reference(site.containerOwner)) && raw(site.capacityRaw)
      && (site.demilitarized === null || unsigned32(site.demilitarized) && site.demilitarized <= 255) && text(site.reason) && strings(site.wings))
    || !text(reply.topologyReason) || !strings(reply.warnings)) throw new Error('The individual fuel consumption detail is malformed.');
  return reply;
}
export function validateFuelDemandHistory(value: unknown): FuelDemandHistoryReply {
  const reply = value as FuelDemandHistoryReply;
  if (!object(value) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !signed32(reply.hour)
    || !reference(reply.country) || !unsigned32(reply.countryInstance) || !signed32(reply.segmentStartHour) || !signed32(reply.window)
    || !span(value) || typeof reply.key !== 'string' || !reply.key || !Array.isArray(reply.runs)
    || !reply.runs.every(run => object(run) && span(run) && typeof run.startsSegment === 'boolean' && raw(run.amountRaw)
      && text(run.reason) && (run.amountRaw === null) === (run.reason !== null) && text(run.recordedName) && text(run.nameReason))
    || !reply.runs.every((run, i) => i === 0 || reply.runs[i - 1].lastHour < run.firstHour) || !strings(reply.warnings)) {
    throw new Error('The individual current fuel occurrence history is malformed.');
  }
  return reply;
}
export function validateFuelDemandStack(value: unknown): FuelDemandStackReply {
  const reply = value as FuelDemandStackReply;
  if (!object(value) || !Array.isArray(reply.parts) || !strings(reply.warnings)
    || !reply.parts.every(part => object(part) && typeof part.key === 'string' && !!part.key && Array.isArray(part.runs))
    || new Set(reply.parts.map(part => part.key)).size !== reply.parts.length) throw new Error('The individual consumption stack is malformed.');
  validateFuelDemandHistory({ ...reply, key: 'header', runs: [] });
  for (const part of reply.parts) validateFuelDemandHistory({ ...reply, key: part.key, runs: part.runs, warnings: [] });
  return reply;
}
/** Only native group/parent links and recorded site occurrence keys select rows. */
export function consumptionParts(reply: FuelDemandReply, level: string): FuelDemandPart[] {
  if (level === 'all') return reply.parts;
  if (/^family:[012]$/.test(level)) return reply.parts.filter(part => part.kind === Number(level.slice(7)));
  const group = reply.commands.find(group => `command:${group.reference}` === level);
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
  const fleet = reply.fleets.find(fleet => `fleet:${fleet.reference}` === level);
  if (fleet) return reply.parts.filter(part => part.kind === 2 && part.parent === fleet.reference);
  const site = reply.sites.find((site, index) => siteLevel(site, index) === level);
  if (site) { const keys = new Set(site.wings); return reply.parts.filter(part => keys.has(part.key)); }
  return [];
}
export function consumptionLevelAvailable(reply: FuelDemandReply, level: string): boolean {
  return level === 'all' || /^family:[012]$/.test(level) || reply.commands.some(group => `command:${group.reference}` === level)
    || reply.fleets.some(fleet => `fleet:${fleet.reference}` === level) || reply.sites.some((site, index) => siteLevel(site, index) === level);
}
export const siteLevel = (site: FuelDemandReply['sites'][number], index: number) => `site:${JSON.stringify([index, site.list, site.ordinal, site.base, site.containerOwner])}`;
export interface FuelConsumptionLevel { campaign: string; branch: string; tag: string; country: string; countryInstance: number; segmentStartHour: number; level: string }
export const fuelConsumptionLevelToken = (level: FuelConsumptionLevel) => setToken(JSON.stringify(level));
export function readFuelConsumptionLevel(token: unknown, campaign: string, branch: string): FuelConsumptionLevel | null {
  if (typeof token !== 'string' || !token) return null;
  try { const value = JSON.parse(contentKey(token));
    if (!object(value) || value.campaign !== campaign || value.branch !== branch || typeof value.tag !== 'string' || !value.tag || !reference(value.country) || !unsigned32(value.countryInstance)
      || !signed32(value.segmentStartHour) || typeof value.level !== 'string' || !value.level) return null;
    const suffix = value.level.slice(value.level.indexOf(':') + 1);
    if (!(value.level === 'all' || /^family:[012]$/.test(value.level)
      || /^(command|fleet):/.test(value.level) && unsigned64(suffix))) {
      if (!value.level.startsWith('site:')) return null;
      const site = JSON.parse(suffix);
      if (!Array.isArray(site) || site.length !== 5 || !Number.isSafeInteger(site[0]) || site[0] < 0 || !unsigned32(site[1]) || site[1] > 255
        || !unsigned32(site[2]) || !unsigned64(site[3]) || !(site[4] === null || reference(site[4]))) return null;
    }
    return value as unknown as FuelConsumptionLevel;
  } catch { return null; }
}
export interface FuelConsumptionSelection {
  campaign: string; branch: string; basis: FuelAwardsParams; key: string; segmentStartHour: number;
}
export const fuelConsumptionToken = (selection: FuelConsumptionSelection) => setToken(JSON.stringify(selection));
export function readFuelConsumption(token: unknown, campaign: string, branch: string): FuelConsumptionSelection | null {
  if (typeof token !== 'string' || !token) return null;
  try {
    const value = JSON.parse(contentKey(token));
    if (!object(value) || value.campaign !== campaign || value.branch !== branch || typeof value.key !== 'string' || !value.key
      || !signed32(value.segmentStartHour) || !object(value.basis) || value.basis.branchId !== branch || typeof value.basis.tag !== 'string' || !value.basis.tag
      || !signed32(value.basis.hour) || value.segmentStartHour > value.basis.hour || !reference(value.basis.country) || !unsigned32(value.basis.countryInstance)) return null;
    return value as unknown as FuelConsumptionSelection;
  } catch { return null; }
}
export function consumptionBasis(reply: Pick<FuelConsumptionReply, 'branchId' | 'tag'>, row: FuelConsumptionRead | null): FuelAwardsParams | null {
  return row !== null && row.countryInstance !== null ? { branchId: reply.branchId, tag: reply.tag, hour: row.firstHour,
    country: row.country, countryInstance: row.countryInstance } : null;
}
export function consumptionSelection(reply: FuelDemandReply, part: FuelDemandPart, campaign: string): FuelConsumptionSelection | null {
  const basis = consumptionBasis(reply, reply.read);
  return basis ? { campaign, branch: reply.branchId, basis, key: part.key, segmentStartHour: reply.read.segmentStartHour } : null;
}
export function consumptionForActor(selection: FuelConsumptionSelection | null | undefined, campaign: string, branch: string, tag?: string,
  aliases?: ReadonlyMap<string, readonly Span[]>): FuelConsumptionSelection | null {
  return selection?.campaign === campaign && selection.branch === branch && selection.basis.branchId === branch
    && (selection.basis.tag === tag && !aliases || tag !== undefined && !!runAt(aliases?.get(selection.basis.tag) ?? [], selection.basis.hour) && !!aliases?.get(tag)?.length) ? selection : null;
}
export function consumptionMatches(selection: FuelConsumptionSelection, row: FuelConsumptionRead | null | undefined) {
  return row?.country === selection.basis.country && row.countryInstance === selection.basis.countryInstance && row.segmentStartHour === selection.segmentStartHour;
}

/** Only contiguous reads of this native actor can extend a selected segment through a rename. */
export function consumptionCoverage(summaries: readonly FuelConsumptionReply[], selection: FuelConsumptionSelection,
  aliases?: ReadonlyMap<string, readonly Span[]>): Span[] {
  if (!aliases) return (summaries.find(reply => reply.tag === selection.basis.tag)?.reads ?? []).filter(read => consumptionMatches(selection, read))
    .map(read => ({ firstHour: read.firstHour, lastHour: read.lastHour }));
  const reads = summaries.flatMap(reply => reply.reads.filter(read => read.country === selection.basis.country && read.countryInstance === selection.basis.countryInstance)
    .map(read => ({ tag: reply.tag, read }))).sort((a, b) => a.read.firstHour - b.read.firstHour);
  if (reads.some((item, index) => index > 0 && reads[index - 1].read.lastHour >= item.read.firstHour)) return [];
  const origin = reads.findIndex(item => item.tag === selection.basis.tag && item.read.firstHour === selection.basis.hour
    && item.read.segmentStartHour === selection.segmentStartHour);
  if (origin < 0) return [];
  const joins = (left: number, right: number) => reads[left].read.lastHour + 1 === reads[right].read.firstHour
    && (reads[left].tag !== reads[right].tag || !reads[right].read.startsSegment);
  let first = origin, last = origin + 1;
  while (first > 0 && joins(first - 1, first)) first--;
  while (last < reads.length && joins(last - 1, last)) last++;
  return reads.slice(first, last).flatMap(({ tag, read }) => clipToSpans([read], aliases.get(tag) ?? []));
}
/** Content reloads keep numeric ownership; no cached name is presented as current. */
export function demandNumbers(reply: FuelDemandReply): FuelDemandReply {
  return { ...reply, warnings: [], parts: reply.parts.map(part => ({ ...part, recordedName: null, nameReason: null })),
    commands: reply.commands.map(group => ({ ...group, recordedName: null, nameReason: null })), fleets: reply.fleets.map(fleet => ({ ...fleet, recordedName: null, nameReason: null })) };
}
export const consumptionHistoryNumbers = (replies: FuelDemandHistoryReply[]) => replies.map(reply => ({ ...reply, warnings: [],
  runs: reply.runs.map(run => ({ ...run, recordedName: null, nameReason: null })) }));
/** Query-basis dates and labels do not own geometry; real supplying run dates do. */
export const consumptionStackNumbers = (replies: FuelDemandStackReply[]) => replies.map(({ hour: _basisHour, warnings: _warnings, ...reply }) => ({ ...reply,
  parts: reply.parts.map(part => ({ ...part, runs: part.runs.map(({ recordedName: _name, nameReason: _nameReason, ...run }) => run) })) }));
/** Adjacent windows may repeat a carried supplying read, with its original date. */
export function consumptionRuns<T extends { firstHour: number; lastHour: number }>(replies: readonly { runs: readonly T[] }[]): T[] {
  const runs = new Map<number, T>();
  for (const reply of replies) for (const run of reply.runs) {
    const previous = runs.get(run.firstHour);
    if (!previous || run.lastHour > previous.lastHour) runs.set(run.firstHour, run);
  }
  return [...runs.values()].sort((a, b) => a.firstHour - b.firstHour);
}
export function stackedConsumptionRuns<T extends { firstHour: number; lastHour: number }>(replies: readonly { parts: readonly { key: string; runs: readonly T[] }[] }[]): Map<string, T[]> {
  const keyed = new Map<string, { runs: readonly T[] }[]>();
  for (const reply of replies) for (const part of reply.parts) {
    const group = keyed.get(part.key);
    if (group) group.push(part); else keyed.set(part.key, [part]);
  }
  return new Map([...keyed].map(([key, parts]) => [key, consumptionRuns(parts)]));
}
export function mergeConsumptionWindows<T extends { window: number }>(previous: readonly T[], incoming: readonly T[], wanted: readonly number[]) {
  const keep = new Set(wanted);
  const replies = new Map(previous.filter(reply => keep.has(reply.window)).map(reply => [reply.window, reply]));
  for (const reply of incoming) if (keep.has(reply.window)) replies.set(reply.window, reply);
  return [...replies.values()].sort((a, b) => a.window - b.window);
}
export function consumptionGeometry(runs: readonly Pick<FuelDemandRun, 'firstHour' | 'lastHour' | 'startsSegment' | 'amountRaw'>[], scale: number) {
  return runs.flatMap((run, i) => run.amountRaw === null ? [] : [{ firstHour: run.firstHour, lastHour: run.lastHour,
    value: Math.max(0, Number(run.amountRaw) / scale), breakBefore: run.startsSegment || i === 0 || runs[i - 1].amountRaw === null || runs[i - 1].lastHour + 1 !== run.firstHour }]);
}
export const consumptionAt = (runs: readonly FuelDemandRun[], hour: number | null) => hour === null ? null : runAt(runs, hour);
