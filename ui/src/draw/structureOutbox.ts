import type { PageReading } from './followed';

export class StructureOutbox {
  private held: PageReading[] = [];
  private waiting = false;
  private readonly send: (updates: PageReading[]) => void;

  constructor(send: (updates: PageReading[]) => void) { this.send = send; }

  add(updates: readonly PageReading[]): void {
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
