import type { Span } from './series';

/** Items spanning hours that may overlap (such as trade deals), found at any hour
 * without scanning them all: a segment tree over the hours where items start and
 * end, built once. A lookup is a binary search for the hour's slab and one walk from
 * the root to it, collecting the items stored on the way, which are exactly those
 * covering the hour. `comparisons` counts the lookups' work, for tests. */
export class IntervalIndex<T extends Span> {
  private readonly bounds: number[];
  private readonly nodes: T[][];
  private readonly slabs: number;
  comparisons = 0;

  constructor(items: readonly T[]) {
    // Slab i holds the hours from bounds[i] up to (not including) bounds[i + 1].
    this.bounds = [...new Set(items.flatMap(item => [item.firstHour, item.lastHour + 1]))].sort((a, b) => a - b);
    this.slabs = Math.max(0, this.bounds.length - 1);
    this.nodes = [];
    for (const item of items) this.insert(1, 0, this.slabs, this.search(item.firstHour), this.search(item.lastHour + 1), item);
  }

  /** The last slab starting at or before `hour`, or -1 before the first. */
  private search(hour: number): number {
    let low = 0;
    let high = this.bounds.length - 1;
    let found = -1;
    while (low <= high) {
      this.comparisons++;
      const middle = (low + high) >> 1;
      if (this.bounds[middle] <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
    }
    return found;
  }

  /** Stores `item` on the nodes that exactly cover slabs [from, to). */
  private insert(node: number, low: number, high: number, from: number, to: number, item: T): void {
    if (to <= low || high <= from) return;
    if (from <= low && high <= to) { (this.nodes[node] ??= []).push(item); return; }
    const middle = (low + high) >> 1;
    this.insert(node * 2, low, middle, from, to, item);
    this.insert(node * 2 + 1, middle, high, from, to, item);
  }

  /** Every item covering `hour`. */
  at(hour: number): T[] {
    const slab = this.search(hour);
    if (slab < 0 || slab >= this.slabs) return [];
    const found: T[] = [];
    let node = 1;
    let low = 0;
    let high = this.slabs;
    for (;;) {
      this.comparisons++;
      const here = this.nodes[node];
      if (here) found.push(...here);
      if (high - low <= 1) return found;
      const middle = (low + high) >> 1;
      if (slab < middle) { node *= 2; high = middle; } else { node = node * 2 + 1; low = middle; }
    }
  }
}

const built = new WeakMap<object, IntervalIndex<Span>>();

/** An index of `items`, built once per `owner` (such as a product reply) and kept
 * while the owner is. */
export function intervalIndexOf<T extends Span>(owner: object, items: () => readonly T[]): IntervalIndex<T> {
  let index = built.get(owner) as IntervalIndex<T> | undefined;
  if (!index) { index = new IntervalIndex(items()); built.set(owner, index as unknown as IntervalIndex<Span>); }
  return index;
}
