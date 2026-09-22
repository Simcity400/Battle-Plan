import type { CountryParams, FocusHistory, FocusOrder, FocusReply } from '../../generated/dashboard';
import type { ProductRequest } from '../../dashboard/client';

export const PRODUCT = 'national-focus';

export function focusRequest(sourceId: string, revision: string, params: CountryParams): ProductRequest<CountryParams> {
  return { sourceId, revision, product: PRODUCT, params };
}

/** Exact lookup over disjoint recorded segments; never carry a value over a gap. */
export function focusAt(history: FocusHistory, hour: number): FocusOrder | null {
  let low = 0, high = history.segments.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (history.segments[middle].firstHour <= hour) low = middle + 1;
    else high = middle;
  }
  const segment = history.segments[low - 1];
  return segment && hour <= segment.lastHour ? segment.order : null;
}

/** Shape checks on the product payload; identity is checked by the transport. */
export function validateFocusReply(input: unknown): FocusReply {
  const reply = input as FocusReply;
  let last = -Infinity;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.history?.segments) && Array.isArray(reply.artwork) && Array.isArray(reply.warnings)
    && reply.history.segments.length <= 100000 && reply.history.segments.every(segment => {
      const ok = Number.isInteger(segment.firstHour) && Number.isInteger(segment.lastHour) && segment.firstHour > last && segment.lastHour >= segment.firstHour
        && segment.order.completed.length <= 512 && segment.order.completed.every((entry, index) => entry.ordinal === index
          && (entry.completedByHour === null || (Number.isInteger(entry.completedByHour) && entry.completedByHour <= segment.firstHour)));
      last = segment.lastHour;
      return ok;
    })
    && reply.artwork.length <= 2048 && reply.artwork.every(({ icon, baseDurationDays }) => (baseDurationDays === null || (Number.isFinite(baseDurationDays) && baseDurationDays >= 0))
      && (icon === null || (icon.length <= 128 * 1024 && /^data:image\/png;base64,[A-Za-z0-9+/]+=*$/.test(icon))));
  if (!valid) throw new Error('Focus history does not match this country and recording.');
  return reply;
}
