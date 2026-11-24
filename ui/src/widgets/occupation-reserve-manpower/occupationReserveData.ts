import type { OccupationReserveReply, ReserveHistoryReply, ReserveSegment } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const country = (value: unknown) => object(value) && Number.isSafeInteger(value.id) && (value.tag === null || typeof value.tag === 'string' && value.tag.length > 0);
const integer = (value: unknown) => Number.isSafeInteger(value);
const warnings = (value: Record<string, unknown>) => Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string');

export function validateOccupationReserve(value: unknown): OccupationReserveReply {
  const ok = object(value) && (value.missing === null || typeof value.missing === 'string') && (value.readHour === null || integer(value.readHour))
    && integer(value.unnamed) && (value.unnamed as number) >= 0 && warnings(value)
    && Array.isArray(value.holders) && value.holders.every(holder => object(holder) && country(holder.country) && typeof holder.admitted === 'boolean'
      && integer(holder.total)
      && Array.isArray(holder.entries) && holder.entries.every(entry => object(entry) && integer(entry.ordinal) && country(entry.source) && integer(entry.amount)));
  if (!ok) throw new Error('The occupation reserve reply is malformed.');
  return value as unknown as OccupationReserveReply;
}

export function validateReserveHistory(value: unknown): ReserveHistoryReply {
  const ok = object(value) && integer(value.hour)
    && (value.nextChange === null || (integer(value.nextChange) && (value.nextChange as number) > (value.hour as number)))
    && warnings(value) && Array.isArray(value.segments) && value.segments.every((segment, index, all) => object(segment)
      && integer(segment.firstHour) && integer(segment.lastHour) && (segment.firstHour as number) <= (segment.lastHour as number)
      && (segment.lastHour as number) <= (value.hour as number)
      && (index === 0 || (all[index - 1] as ReserveSegment).lastHour < (segment.firstHour as number)) && typeof segment.admitted === 'boolean'
      && (segment.entries === null ? typeof segment.unknown === 'string' : segment.unknown === null && Array.isArray(segment.entries)
        && segment.entries.every(entry => object(entry) && integer(entry.source) && integer(entry.amount))))
    && Array.isArray(value.sources) && value.sources.every(source => object(source) && integer(source.id) && Array.isArray(source.tags)
      && source.tags.every(tag => object(tag) && (tag.tag === null || typeof tag.tag === 'string')
        && integer(tag.firstHour) && integer(tag.lastHour) && (tag.lastHour as number) <= (value.hour as number)));
  if (!ok) throw new Error('The occupation reserve history is malformed.');
  return value as unknown as ReserveHistoryReply;
}

export function reserveHistoryProof(data: unknown, _requested: readonly [number, number], at: number): readonly [number, number] | null {
  const reply = data as ReserveHistoryReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= at && at <= last ? [reply.hour, last] : null;
}

export const persons = (value: number) => value.toLocaleString('en-US');

export function sourceTagAt(history: ReserveHistoryReply, source: number, hour: number): string | null {
  const tags = history.sources.find(item => item.id === source)?.tags ?? [];
  return tags.find(tag => tag.firstHour <= hour && hour <= tag.lastHour)?.tag ?? null;
}

export const segmentAt = (history: ReserveHistoryReply, hour: number): ReserveSegment | null =>
  history.segments.find(segment => segment.firstHour <= hour && hour <= segment.lastHour) ?? null;

export function segmentParts(segment: ReserveSegment, sources: readonly number[]): (number | undefined)[] {
  if (!segment.entries) return sources.map(() => undefined);
  const by = new Map<number, number>();
  for (const entry of segment.entries) if (entry.amount > 0) by.set(entry.source, (by.get(entry.source) ?? 0) + entry.amount);
  return sources.map(source => by.get(source) ?? (segment.admitted ? 0 : undefined));
}

export function holderOf(token: unknown): { tag: string; id: number } | null {
  if (typeof token !== 'string') return null;
  const [tag, id] = token.split('|');
  const raw = Number(id);
  return tag && id !== undefined && Number.isSafeInteger(raw) ? { tag, id: raw } : null;
}
