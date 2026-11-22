import type { OrderedReading, ReadingOrder } from '../../draw/followed';

export const newer = (order: ReadingOrder, than: ReadingOrder | undefined) => !than || (order.generation !== than.generation
  ? order.generation > than.generation : order.from !== than.from ? order.from > than.from : order.sequence > than.sequence);

export class StructureQueue {
  private due: OrderedReading[] = [];
  private readonly shown: Map<number, ReadingOrder>;

  constructor(shown = new Map<number, ReadingOrder>()) { this.shown = shown; }

  get pending() { return this.due.length > 0; }

  add(read: OrderedReading) {
    let lo = 0, hi = this.due.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.due[mid]!.hour > read.hour) lo = mid + 1; else hi = mid;
    }
    this.due.splice(lo, 0, read);
  }

  discard(ids: ReadonlySet<number>, from = -Infinity) {
    this.due = this.due.filter(r => !ids.has(r.id) || r.hour < from);
  }

  forget(id: number) {
    this.discard(new Set([id]));
    this.shown.delete(id);
  }

  take(read: OrderedReading): boolean {
    if (!newer(read.order, this.shown.get(read.id))) return false;
    this.shown.set(read.id, read.order);
    return true;
  }

  drain(hour: number, publish: (read: OrderedReading) => void) {
    if (!this.due.length || this.due.at(-1)!.hour > hour) return;
    const eligible = new Map<number, OrderedReading>();
    while (this.due.length && this.due.at(-1)!.hour <= hour) {
      const read = this.due.pop()!, held = eligible.get(read.id);
      if (!held || newer(read.order, held.order)) eligible.set(read.id, read);
    }
    for (const read of eligible.values()) publish(read);
  }
}
