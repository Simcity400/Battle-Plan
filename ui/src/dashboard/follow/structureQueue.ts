import type { PageReading } from '../../draw/followed';

export class StructureQueue {
  private due: PageReading[] = [];

  get pending() { return this.due.length > 0; }

  add(read: PageReading) {
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

  drain(hour: number, publish: (read: PageReading) => void) {
    if (!this.due.length || this.due.at(-1)!.hour > hour) return;
    const eligible = new Map<number, PageReading>();
    while (this.due.length && this.due.at(-1)!.hour <= hour) {
      const read = this.due.pop()!;
      eligible.set(read.id, read);
    }
    for (const read of eligible.values()) publish(read);
  }
}
