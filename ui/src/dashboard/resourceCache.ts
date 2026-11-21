export class ResourceCache<T> {
  private entries = new Map<string, { controller: AbortController; promise: Promise<T>; users: number; settled: boolean; bytes: number; value?: T }>();
  private pins = new Map<object, ReadonlySet<string>>();
  private pinned = new Map<string, number>();
  private maxEntries: number;
  private maxBytes: number;
  private size: (value: T) => number;
  constructor(maxEntries: number, maxBytes: number, size: (value: T) => number) {
    this.maxEntries = maxEntries; this.maxBytes = maxBytes; this.size = size;
  }

  get(key: string, load: (signal: AbortSignal) => Promise<T>, signal: AbortSignal, retain: (value: T) => boolean = () => true): Promise<T> {
    if (signal.aborted) return Promise.reject(new DOMException('Request cancelled', 'AbortError'));
    let entry = this.entries.get(key);
    if (!entry) {
      const controller = new AbortController();
      entry = { controller, promise: Promise.resolve().then(() => load(controller.signal)), users: 0, settled: false, bytes: 0 };
      const created = entry;
      this.entries.set(key, created);
      created.promise = created.promise.then(value => {
        created.settled = true;
        created.value = value;
        if (!retain(value)) {
          if (this.entries.get(key) === created) this.entries.delete(key);
          return value;
        }
        created.bytes = this.size(value);
        this.trim();
        return value;
      }, error => {
        if (this.entries.get(key) === created) this.entries.delete(key);
        throw error;
      });
    }
    const current = entry;
    this.entries.delete(key); this.entries.set(key, current);
    current.users++;
    return new Promise<T>((resolve, reject) => {
      let finished = false;
      const finish = (callback: () => void) => {
        if (finished) return;
        finished = true;
        signal.removeEventListener('abort', abort);
        current.users--;
        callback();
        queueMicrotask(() => {
          if (!current.users && !current.settled) {
            if (this.entries.get(key) === current) this.entries.delete(key);
            current.controller.abort();
          }
        });
      };
      const abort = () => finish(() => reject(new DOMException('Request cancelled', 'AbortError')));
      signal.addEventListener('abort', abort, { once: true });
      current.promise.then(value => finish(() => resolve(value)), error => finish(() => reject(error)));
    });
  }

  holds(key: string): boolean { return this.entries.has(key); }

  getSettled(key: string): Promise<T> | undefined {
    const entry = this.entries.get(key);
    if (!entry?.settled) return undefined;
    this.entries.delete(key); this.entries.set(key, entry);
    return entry.promise;
  }

  peek(key: string): T | undefined {
    return this.entries.get(key)?.value;
  }

  pin(owner: object, keys: Iterable<string>): void {
    this.unpin(owner);
    const held = new Set(keys);
    this.pins.set(owner, held);
    for (const key of held) this.pinned.set(key, (this.pinned.get(key) ?? 0) + 1);
  }

  unpin(owner: object): void {
    const held = this.pins.get(owner);
    if (!held) return;
    this.pins.delete(owner);
    for (const key of held) {
      const count = (this.pinned.get(key) ?? 1) - 1;
      if (count > 0) this.pinned.set(key, count); else this.pinned.delete(key);
    }
    this.trim();
  }

  deleteSource(sourceId: string): void {
    for (const [key, entry] of this.entries) if (JSON.parse(key)[0] === sourceId) {
      this.entries.delete(key); entry.controller.abort();
    }
    for (const [owner, held] of [...this.pins]) {
      const kept = [...held].filter(key => JSON.parse(key)[0] !== sourceId);
      if (kept.length < held.size) this.pin(owner, kept);
    }
  }

  deleteSettled(sourceId: string): void {
    for (const [key, entry] of this.entries) if (entry.settled && JSON.parse(key)[0] === sourceId) this.entries.delete(key);
  }

  private trim(): void {
    let bytes = 0;
    let count = 0;
    for (const [key, entry] of this.entries) if (entry.settled && !this.pinned.has(key)) { bytes += entry.bytes; count++; }
    for (const [key, entry] of this.entries) {
      if (count <= this.maxEntries && bytes <= this.maxBytes) break;
      if (entry.settled && !this.pinned.has(key)) { this.entries.delete(key); bytes -= entry.bytes; count--; }
    }
  }
}
