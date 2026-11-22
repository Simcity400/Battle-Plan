import { createContext } from 'react';

export class OutsideHeads {
  private readonly heads = new Map<object, boolean>();
  private readonly listeners = new Set<() => void>();
  private inside = 0;
  private shown = false;

  set(head: object, outside: boolean | null): void {
    if (this.heads.get(head) === false) this.inside--;
    if (outside === null) this.heads.delete(head);
    else {
      this.heads.set(head, outside);
      if (!outside) this.inside++;
    }
    const next = this.heads.size > 0 && this.inside === 0;
    if (next === this.shown) return;
    this.shown = next;
    for (const listener of [...this.listeners]) listener();
  }

  readonly outside = (): boolean => this.shown;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
}

export const OutsideHeadsContext = createContext<OutsideHeads | null>(null);
