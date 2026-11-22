import type { FollowUpdate, PageReading } from '../../draw/followed';

export function readingAt(update: FollowUpdate, hour: number): PageReading {
  let reading = update.now;
  for (const lead of update.ahead) {
    if (lead.hour > hour) break;
    reading = lead;
  }
  return reading;
}

export class LatestReadings {
  private readonly latest = new Map<number, FollowUpdate>();
  private readonly shown: Map<number, PageReading>;
  private due = new Set<number>();
  private next = Infinity;

  constructor(shown = new Map<number, PageReading>()) { this.shown = shown; }

  get waiting() { return this.due.size > 0; }

  land(update: FollowUpdate, hour: number, show: (reading: PageReading) => void) {
    this.latest.set(update.now.id, update);
    this.place(update, hour, show);
  }

  advance(hour: number, show: (reading: PageReading) => void) {
    if (hour < this.next) return;
    const due = this.due;
    this.due = new Set();
    this.next = Infinity;
    for (const id of due) {
      const update = this.latest.get(id);
      if (update) this.place(update, hour, show);
    }
  }

  forget(ids: Iterable<number>) {
    for (const id of ids) { this.latest.delete(id); this.shown.delete(id); this.due.delete(id); }
  }

  private place(update: FollowUpdate, hour: number, show: (reading: PageReading) => void) {
    const id = update.now.id, reading = readingAt(update, hour);
    if (this.shown.get(id) !== reading) { this.shown.set(id, reading); show(reading); }
    const later = update.ahead.find(lead => lead.hour > hour);
    if (!later) { this.due.delete(id); return; }
    this.due.add(id);
    if (later.hour < this.next) this.next = later.hour;
  }
}
