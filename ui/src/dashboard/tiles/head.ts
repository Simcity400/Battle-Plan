import type { ChartSummary } from '../../charts/chartSummary';

type Line = Pick<ChartSummary, 'from' | 'buckets'>;

/** Lines whose value at the head is followed: their readers by the line's head key, the newest last. Keyed by the
 * line, not by one assembly of it, so a tile landing while playing needs no new registration. */
const followed = new Map<string, (() => number | null | undefined)[]>();

/** The head key of tag `tag`'s line in the series whose tiles are named `lane`. */
export const tileHead = (lane: string, tag: string) => `${lane}\u0000${tag}`;

/** The value at the head of the line `head` names is `read()` while it is followed: a number, null where the
 * followed answer holds none, undefined until its first answer. The returned function stops it. */
export function followAtHead(head: string, read: () => number | null | undefined): () => void {
  let readers = followed.get(head);
  if (!readers) followed.set(head, readers = []);
  readers.push(read);
  return () => {
    const held = followed.get(head);
    const at = held?.lastIndexOf(read) ?? -1;
    if (at < 0) return;
    held!.splice(at, 1);
    if (!held!.length) followed.delete(head);
  };
}

/** The value a tiled line is drawn at from its head bin's first hour to the head: the followed value at the head
 * when the line is followed; otherwise the last value of the bin before the head's, when the tiles hold it; null
 * where that bin holds none (nothing is drawn past a gap). */
export function valueAtHead(head: string, summary: Line, binHours: number, hour: number): number | null {
  const value = followed.get(head)?.at(-1)?.();
  if (value !== undefined) return value;
  const slot = Math.floor(hour / binHours) - Math.floor(summary.from / binHours) - 1;
  return slot >= 0 && slot < summary.buckets.length ? summary.buckets[slot]?.lastValue ?? null : null;
}
