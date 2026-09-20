import type { FocusReply, FocusRequest } from './client';
import type { FocusHistory, FocusOrder } from '../generated/analytics';

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

/** Conservative JS storage estimate, including strings, rows and decoded thumbnails. */
export function focusReplyBytes(reply: FocusReply): number {
  const text = (s: string | null) => 32 + (s?.length ?? 0) * 2;
  return 256 + text(reply.sourceId) + text(reply.revision) + text(reply.branchId) + text(reply.tag)
    + reply.history.segments.reduce((bytes, { order }) => bytes + 256 + text(order.countryName) + text(order.current) + text(order.continuous)
      + order.completed.reduce((sum, row) => sum + 64 + text(row.token), 0), 0)
    + reply.artwork.reduce((bytes, art) => bytes + 128 + text(art.token) + text(art.name) + text(art.description) + text(art.icon)
      + (art.icon ? 96 * 96 * 4 : 0), 0)
    + reply.warnings.reduce((bytes, warning) => bytes + text(warning), 0);
}

export function validateFocusReply(reply: FocusReply, query: FocusRequest): FocusReply {
  let last = -Infinity;
  const valid = reply.sourceId === query.sourceId && reply.revision === query.revision && reply.branchId === query.branchId && reply.tag === query.tag
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
