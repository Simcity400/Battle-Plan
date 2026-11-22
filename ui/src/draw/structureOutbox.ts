import type { OrderedReading } from './followed';

export class StructureOutbox {
  private held: OrderedReading[] = [];
  private waiting = false;
  private readonly send: (updates: OrderedReading[]) => void;

  constructor(send: (updates: OrderedReading[]) => void) { this.send = send; }

  add(updates: readonly OrderedReading[]): void {
    if (!updates.length) return;
    const newer = new Set(updates.map(update => update.id));
    this.held = [...this.held.filter(update => !newer.has(update.id)), ...updates];
    this.flush();
  }

  taken(): void {
    this.waiting = false;
    this.flush();
  }

  private flush(): void {
    if (this.waiting || !this.held.length) return;
    const updates = this.held;
    this.held = [];
    this.waiting = true;
    this.send(updates);
  }
}
