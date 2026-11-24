export class CadenceGate {
  private readonly intervalMs: number;
  private lastAt = -Infinity;
  private lastKey = '';
  private pending = true;
  private emitted = false;

  constructor(intervalMs: number) {
    this.intervalMs = Math.max(1, intervalMs);
  }

  invalidate(): void {
    this.pending = true;
  }

  due(now: number, speed: number, key: string, force = false): boolean {
    const keyChanged = key !== this.lastKey;
    if (keyChanged) this.pending = true;
    if (!this.emitted) return true;
    if (!this.pending) return false;
    return force || speed === 0 || keyChanged || now - this.lastAt >= this.intervalMs;
  }

  record(now: number, key: string): void {
    this.lastAt = now;
    this.lastKey = key;
    this.pending = false;
    this.emitted = true;
  }

  reset(): void {
    this.lastKey = '';
    this.pending = false;
    this.emitted = false;
  }

  get dirty(): boolean {
    return this.pending;
  }

  remainingMs(now: number): number {
    if (!this.pending) return Infinity;
    return Math.max(1, this.intervalMs - (now - this.lastAt));
  }
}

export interface CadenceTimers {
  readonly set: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>;
  readonly clear: (id: ReturnType<typeof setTimeout>) => void;
}

const DEFAULT_TIMERS: CadenceTimers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: id => clearTimeout(id),
};

export class TrailingFlushScheduler {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private dueAt = Infinity;
  private onFlush: () => void = () => {};
  private readonly gates: readonly CadenceGate[];
  private readonly timers: CadenceTimers;

  constructor(gates: readonly CadenceGate[], timers: CadenceTimers = DEFAULT_TIMERS) {
    this.gates = gates;
    this.timers = timers;
  }

  sync(now: number, speed: number, onFlush: () => void): void {
    let waitMs = Infinity;
    if (speed > 0) {
      for (const gate of this.gates) {
        const ms = gate.remainingMs(now);
        if (ms < waitMs) waitMs = ms;
      }
    }
    if (!Number.isFinite(waitMs)) {
      this.cancel();
      return;
    }
    this.onFlush = onFlush;
    const dueAt = now + waitMs;
    if (this.timer !== null && this.dueAt <= dueAt) return;
    this.cancel();
    this.dueAt = dueAt;
    this.timer = this.timers.set(() => {
      this.timer = null;
      this.dueAt = Infinity;
      this.onFlush();
    }, waitMs);
    (this.timer as unknown as { unref?: () => void })?.unref?.();
  }

  cancel(): void {
    if (this.timer !== null) {
      this.timers.clear(this.timer);
      this.timer = null;
      this.dueAt = Infinity;
    }
  }
}
