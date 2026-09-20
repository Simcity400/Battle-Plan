/** Bounded immutable results with shared requests and per-consumer cancellation. */
export class ResourceCache<T> {
  private entries = new Map<string, { controller: AbortController; promise: Promise<T>; users: number; settled: boolean; bytes: number }>();
  private maxEntries: number;
  private maxBytes: number;
  private size: (value: T) => number;
  constructor(maxEntries: number, maxBytes: number, size: (value: T) => number) {
    this.maxEntries = maxEntries; this.maxBytes = maxBytes; this.size = size;
  }

  get(key: string, load: (signal: AbortSignal) => Promise<T>, signal: AbortSignal): Promise<T> {
    if (signal.aborted) return Promise.reject(new DOMException('Request cancelled', 'AbortError'));
    let entry = this.entries.get(key);
    if (!entry) {
      const controller = new AbortController();
      entry = { controller, promise: Promise.resolve().then(() => load(controller.signal)), users: 0, settled: false, bytes: 0 };
      const created = entry;
      this.entries.set(key, created);
      created.promise = created.promise.then(value => {
        created.settled = true;
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
        // React effect replay can resubscribe before cancelling shared work.
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

  deleteSource(sourceId: string): void {
    for (const [key, entry] of this.entries) if (JSON.parse(key)[0] === sourceId) {
      this.entries.delete(key); entry.controller.abort();
    }
  }

  private trim(): void {
    let bytes = 0;
    let count = 0;
    for (const entry of this.entries.values()) if (entry.settled) { bytes += entry.bytes; count++; }
    for (const [key, entry] of this.entries) {
      if (count <= this.maxEntries && bytes <= this.maxBytes) break;
      if (entry.settled) { this.entries.delete(key); bytes -= entry.bytes; count--; }
    }
  }
}
