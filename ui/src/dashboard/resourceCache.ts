/** Bounded immutable results with shared requests and per-consumer cancellation. */
export class ResourceCache<T> {
  private entries = new Map<string, { controller: AbortController; promise: Promise<T>; users: number; settled: boolean; bytes: number }>();
  private displacedLoads = 0;
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

  /** Mounted progressive reads can reuse only final results, never another mount's polling. */
  getSettled(key: string): Promise<T> | undefined {
    const entry = this.entries.get(key);
    if (!entry?.settled) return undefined;
    this.entries.delete(key); this.entries.set(key, entry);
    return entry.promise;
  }

  /** Publish a final result without replacing the owners of any outstanding load. */
  put(key: string, value: T): void {
    const previous = this.entries.get(key);
    if (previous && !previous.settled) {
      // Keep the displaced one-shot load reachable by source cancellation until
      // its existing consumers finish. Its reply cannot replace the final value.
      const pendingKey = JSON.stringify([JSON.parse(key)[0], key, ++this.displacedLoads]);
      this.entries.set(pendingKey, previous);
      const finished = () => { this.entries.delete(pendingKey); };
      void previous.promise.then(finished, finished);
    }
    this.entries.delete(key);
    this.entries.set(key, { controller: new AbortController(), promise: Promise.resolve(value),
      users: 0, settled: true, bytes: this.size(value) });
    this.trim();
  }

  /** Forget every entry of a source, cancelling loads still in flight. */
  deleteSource(sourceId: string): void {
    for (const [key, entry] of this.entries) if (JSON.parse(key)[0] === sourceId) {
      this.entries.delete(key); entry.controller.abort();
    }
  }

  /** Forget only completed entries of a source; loads in flight keep their consumers. */
  deleteSettled(sourceId: string): void {
    for (const [key, entry] of this.entries) if (entry.settled && JSON.parse(key)[0] === sourceId) this.entries.delete(key);
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
