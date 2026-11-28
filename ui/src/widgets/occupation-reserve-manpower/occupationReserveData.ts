import type { OccupationReserveReply, ReserveHistoryReply, ReserveSegment } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const country = (value: unknown) => object(value) && Number.isSafeInteger(value.id) && (value.tag === null || typeof value.tag === 'string' && value.tag.length > 0);
const integer = (value: unknown) => Number.isSafeInteger(value);
const warnings = (value: Record<string, unknown>) => Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string');

export function validateOccupationReserve(value: unknown): OccupationReserveReply {
  const ok = object(value) && (value.missing === null || typeof value.missing === 'string') && (value.readHour === null || integer(value.readHour))
    && integer(value.unnamed) && (value.unnamed as number) >= 0 && (value.noneMissing === null || typeof value.noneMissing === 'string') && warnings(value)
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

export function reserveHistoryProvenLast(history: ReserveHistoryReply): number {
  return history.nextChange === null ? Number.MAX_SAFE_INTEGER : history.nextChange - 1;
}

export const persons = (value: number) => value.toLocaleString('en-US');

export function sourceTagAt(history: ReserveHistoryReply, source: number, hour: number): string | null {
  const tags = history.sources.find(item => item.id === source)?.tags ?? [];
  const provenLast = reserveHistoryProvenLast(history);
  return tags.find(tag => tag.firstHour <= hour && hour <= (tag.lastHour === history.hour ? provenLast : tag.lastHour))?.tag ?? null;
}

export const segmentAt = (history: ReserveHistoryReply, hour: number): ReserveSegment | null => {
  const provenLast = reserveHistoryProvenLast(history);
  return history.segments.find(segment => segment.firstHour <= hour && hour <= (segment.lastHour === history.hour ? provenLast : segment.lastHour)) ?? null;
};

export const UNADMITTED_RESERVE_REASON = 'Occupation settings were not recorded at this hour';

export type ReserveValue =
  | { readonly kind: 'known'; readonly value: number }
  | { readonly kind: 'zero'; readonly value: 0 }
  | { readonly kind: 'notRecorded'; readonly reason: string };

export function segmentParts(segment: ReserveSegment, sources: readonly number[]): ReserveValue[] {
  if (segment.entries === null) {
    const reason = segment.unknown ?? 'Not recorded';
    return sources.map(() => ({ kind: 'notRecorded', reason }));
  }
  const by = new Map<number, number>();
  for (const entry of segment.entries) {
    if (entry.amount > 0) by.set(entry.source, (by.get(entry.source) ?? 0) + entry.amount);
    else if (!by.has(entry.source)) by.set(entry.source, 0);
  }
  return sources.map(source => {
    const amount = by.get(source);
    if (amount !== undefined) return amount > 0 ? { kind: 'known', value: amount } : { kind: 'zero', value: 0 };
    return segment.admitted ? { kind: 'zero', value: 0 } : { kind: 'notRecorded', reason: UNADMITTED_RESERVE_REASON };
  });
}

export function segmentTotal(segment: ReserveSegment, parts: readonly ReserveValue[]): ReserveValue {
  if (segment.entries === null) return { kind: 'notRecorded', reason: segment.unknown ?? 'Not recorded' };
  let sum = 0;
  for (const part of parts) {
    if (part.kind === 'notRecorded') return part;
    sum += part.value;
  }
  if (parts.length === 0 && !segment.admitted) return { kind: 'notRecorded', reason: UNADMITTED_RESERVE_REASON };
  return sum === 0 ? { kind: 'zero', value: 0 } : { kind: 'known', value: sum };
}

export function holderOf(token: unknown): { tag: string; id: number } | null {
  if (typeof token !== 'string') return null;
  const [tag, id] = token.split('|');
  const raw = Number(id);
  return tag && id !== undefined && Number.isSafeInteger(raw) ? { tag, id: raw } : null;
}
