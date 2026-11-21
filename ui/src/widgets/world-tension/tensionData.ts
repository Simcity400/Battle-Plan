import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { EventBadge } from '../../charts/badges';
import type { TensionContribution, ThreatEvent, ThreatEventsReply, WorldTensionReply } from '../../generated/dashboard';

export const WORLD = 'World';
export const WORLD_ROW = 'world';
export const TENSION_DECIMALS = 3;
export const CONTRIBUTION_DECIMALS = 5;
export const OTHER = '\u0000other';

const raw = (value: unknown) => typeof value === 'string' && /^-?\d+$/.test(value);
const integer = Number.isSafeInteger;
const tagOk = (value: unknown) => value === null || typeof value === 'string';

export function validateWorldTension(value: unknown): WorldTensionReply {
  const reply = value as Partial<WorldTensionReply> | null;
  const ok = !!reply && typeof reply === 'object' && integer(reply.tensionScale) && integer(reply.contributionScale) && raw(reply.ceilingRaw)
    && typeof reply.unnamed === 'boolean' && Array.isArray(reply.warnings) && (reply.missing === null || typeof reply.missing === 'string')
    && Array.isArray(reply.contributions) && reply.contributions.every(item => item && integer(item.slot) && tagOk(item.tag) && typeof item.remembered === 'boolean')
    && Array.isArray(reply.sources) && reply.sources.every(item => item && integer(item.ordinal) && tagOk(item.tag) && typeof item.reason === 'string'
      && raw(item.finalRaw) && raw(item.dailyRaw) && integer(item.date));
  if (!ok) throw new Error('The world tension reply is malformed.');
  return reply as WorldTensionReply;
}

export function validateThreatEvents(value: unknown): ThreatEventsReply {
  const reply = value as Partial<ThreatEventsReply> | null;
  const ok = !!reply && typeof reply === 'object' && Array.isArray(reply.warnings) && Array.isArray(reply.missing)
    && Array.isArray(reply.events) && reply.events.every(event => event && integer(event.hour) && tagOk(event.tag) && typeof event.reason === 'string'
      && integer(event.firstRead) && integer(event.lastRead) && event.firstRead <= event.lastRead && typeof event.open === 'boolean'
      && Array.isArray(event.added) && event.added.length > 0 && event.added.every(added => added && integer(added.hour) && raw(added.addedRaw)));
  if (!ok) throw new Error('The threat events reply is malformed.');
  return reply as ThreatEventsReply;
}

export function percent(rawValue: string, scale: number, digits = 1): string {
  const decimals = Math.round(Math.log10(scale));
  return `${formatRaw(rawValue, decimals, digits)}%`;
}

export const percentOf = (units: number, scale: number, digits = 1) => percent(String(Math.trunc(units)), scale, digits);

export const contributorName = (tag: string | null, remembered: boolean, name: (tag: string) => string) =>
  tag === null ? 'Unknown country' : remembered ? `${name(tag)} (no longer exists)` : name(tag);

export const TENSION_CELL = liveKey(WORLD_ROW, 'tension');
export const SUM_CELL = liveKey(WORLD_ROW, 'sum');
export const UNNAMED_CELL = liveKey(WORLD_ROW, 'unnamed');
export const contributionCell = (slot: number) => liveKey(String(slot), 'value');
export const sourceCell = (ordinal: number) => liveKey(`source${ordinal}`, 'current');

export function shareUnits(value: bigint, tension: bigint, sum: bigint, scalarDivisor = 100n): bigint {
  if (sum <= 0n) return 0n;
  const product = value * tension;
  const quotient = product / sum, remainder = product % sum;
  return quotient * scalarDivisor + remainder * scalarDivisor / sum;
}

export interface WorldCells { tension: number | null; sum: number | null; unnamed: number; why: string }

export function worldCells(values: LiveValues<number>, unnamed: boolean): WorldCells {
  const tension = values.get(TENSION_CELL) ?? null, sum = values.get(SUM_CELL) ?? null;
  return { tension, sum, unnamed: unnamed ? values.get(UNNAMED_CELL) ?? 0 : 0,
    why: tension === null ? values.missing(TENSION_CELL) : sum === null ? values.missing(SUM_CELL) : '' };
}

export function layerUnits(value: number | null, world: WorldCells, share: boolean): number | null {
  if (value === null) return null;
  if (!share) return value;
  if (world.tension === null || world.sum === null) return null;
  return Number(shareUnits(BigInt(Math.trunc(value)), BigInt(Math.trunc(world.tension)), BigInt(Math.trunc(world.sum))));
}

export function otherUnits(listed: readonly (number | null)[], world: WorldCells, share: boolean): number | null {
  if (listed.some(value => value === null)) return null;
  const whole = share ? (world.tension === null ? null : world.tension * 100) : world.sum;
  return whole === null ? null : whole - listed.reduce<number>((sum, value) => sum + value!, 0);
}

export function layerContributions(contributions: readonly TensionContribution[], count: number, keep: (tag: string) => boolean): TensionContribution[] {
  const out: TensionContribution[] = [];
  for (const item of contributions) {
    if (out.length >= count) break;
    if (item.tag !== null && keep(item.tag)) out.push(item);
  }
  return out;
}

function scaled(bucket: ChartBucket, first: number, last: number): ChartBucket {
  return { ...bucket, firstValue: bucket.firstValue * first, lastValue: bucket.lastValue * last,
    minValue: bucket.minValue * Math.min(first, last), maxValue: bucket.maxValue * Math.max(first, last) };
}

export function shareSummary(part: ChartSummary, tension: ChartSummary, sum: ChartSummary): ChartSummary {
  const factor = (t: number, s: number) => s > 0 ? t / s : 0;
  const buckets = part.buckets.map((bucket, at) => {
    const t = tension.buckets[at], s = sum.buckets[at];
    return bucket && t && s ? scaled(bucket, factor(t.firstValue, s.firstValue), factor(t.lastValue, s.lastValue)) : null;
  });
  const { runs: _runs, tiles, ...rest } = part;
  const [t, s] = [tension.tiles, sum.tiles];
  const head = tiles && { ...tiles, valueAt: (hour: number) => {
    const [value, over, under] = [tiles.valueAt(hour), t?.valueAt(hour) ?? null, s?.valueAt(hour) ?? null];
    return value === null || over === null || under === null ? null : value * factor(over, under);
  } };
  return { ...rest, buckets, ...(head ? { tiles: head } : {}) };
}

export function shareSummaries(): (part: ChartSummary, tension: ChartSummary, sum: ChartSummary) => ChartSummary {
  const made = new WeakMap<ChartSummary, { tension: ChartSummary; sum: ChartSummary; share: ChartSummary }>();
  return (part, tension, sum) => {
    const held = made.get(part);
    if (held && held.tension === tension && held.sum === sum) return held.share;
    const share = shareSummary(part, tension, sum);
    made.set(part, { tension, sum, share });
    return share;
  };
}

export function eventBadges(events: readonly ThreatEvent[], from: number, to: number, colour: (tag: string | null) => string | undefined): Map<EventBadge, ThreatEvent> {
  return new Map(events.filter(event => event.hour >= from && event.hour <= to)
    .map(event => [{ hour: event.hour, glyph: '▼', label: event.reason, color: colour(event.tag), shape: 'triangle' as const }, event]));
}

export function eventsThrough(reply: ThreatEventsReply, hour: number): ThreatEvent[] {
  return reply.events.filter(event => event.firstRead <= hour).sort((a, b) => b.firstRead - a.firstRead || b.hour - a.hour);
}

export function addedAt(event: ThreatEvent, hour: number): string {
  let shown = event.added[0].addedRaw;
  for (const added of event.added) if (added.hour <= hour) shown = added.addedRaw;
  return shown;
}

export function eventGroup(events: readonly ThreatEvent[]): { totalRaw: string; largest: ThreatEvent[] } {
  const total = events.reduce((sum, event) => sum + BigInt(event.added[0].addedRaw), 0n);
  const largest = [...events].sort((a, b) => {
    const left = BigInt(a.added[0].addedRaw), right = BigInt(b.added[0].addedRaw);
    return right > left ? 1 : right < left ? -1 : a.reason.localeCompare(b.reason);
  });
  return { totalRaw: total.toString(), largest };
}

export function eventKeys(events: readonly ThreatEvent[]): string[] {
  const seen = new Map<string, number>();
  return events.map(event => {
    const alike = JSON.stringify([event.hour, event.firstRead, event.reason]);
    const place = seen.get(alike) ?? 0;
    seen.set(alike, place + 1);
    return `${alike}#${place}`;
  });
}

export function causeAt(event: ThreatEvent, hour: number): string | null {
  const limit = Math.min(Math.floor(hour), event.lastRead);
  return event.firstRead <= limit ? addedAt(event, limit) : null;
}

export function shareText(value: number, sum: number | null): string {
  if (sum === null || sum === 0) return 'Not known';
  return `${(Math.round(value / Math.abs(sum) * 10_000) / 100).toFixed(2)}%`;
}
