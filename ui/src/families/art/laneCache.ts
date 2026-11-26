import type { ArtBitmap, ArtResolution, ArtResult } from './types.ts';

export type LaneArtFetcher<TBitmap extends ArtBitmap = ImageBitmap> = (
  url: string,
  signal: AbortSignal,
) => Promise<TBitmap>;

export interface LaneArtCacheOptions<TBitmap extends ArtBitmap = ImageBitmap> {
  readonly maxEntries?: number;
  readonly maxFailed?: number;
  readonly maxConcurrent?: number;
  readonly contentRevision?: number;
  readonly fetchBitmap?: LaneArtFetcher<TBitmap>;
  readonly pool?: LaneArtPool<TBitmap>;
  readonly onSettle?: () => void;
}

const DEFAULT_MAX_ENTRIES = 128;
const DEFAULT_MAX_FAILED = 256;
const DEFAULT_MAX_CONCURRENT = 6;
const DEFAULT_MAX_MEASURED = 8192;

async function defaultFetchBitmap(url: string, signal: AbortSignal): Promise<ImageBitmap> {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error(`art ${response.status}`);
  const blob = await response.blob();
  if (signal.aborted) throw new DOMException('Aborted', 'AbortError');
  return createImageBitmap(blob);
}

export interface LaneArtClaim<TBitmap extends ArtBitmap = ImageBitmap> {
  readonly bitmap: Promise<TBitmap>;
  release(): void;
}

interface Pooled<TBitmap extends ArtBitmap> {
  users: number;
  ready: TBitmap | null;
  readonly controller: AbortController;
  readonly bitmap: Promise<TBitmap>;
}

export class LaneArtPool<TBitmap extends ArtBitmap = ImageBitmap> {
  private readonly entries = new Map<string, Pooled<TBitmap>>();

  private readonly fetchBitmap: LaneArtFetcher<TBitmap>;

  constructor(fetchBitmap: LaneArtFetcher<TBitmap> = defaultFetchBitmap as LaneArtFetcher<TBitmap>) {
    this.fetchBitmap = fetchBitmap;
  }

  acquire(url: string, revision: number): LaneArtClaim<TBitmap> {
    const key = `${revision}\u0000${url}`;
    let entry = this.entries.get(key);
    if (!entry) {
      const controller = new AbortController();
      const made: Pooled<TBitmap> = { users: 0, ready: null, controller, bitmap: this.fetchBitmap(url, controller.signal) };
      made.bitmap.then(
        bitmap => { if (this.entries.get(key) === made) made.ready = bitmap; else bitmap.close(); },
        () => { if (this.entries.get(key) === made) this.entries.delete(key); },
      );
      this.entries.set(key, entry = made);
    }
    const held = entry;
    held.users++;
    let released = false;
    return {
      bitmap: held.bitmap,
      release: () => {
        if (released) return;
        released = true;
        if (--held.users > 0 || this.entries.get(key) !== held) return;
        this.entries.delete(key);
        if (held.ready) held.ready.close();
        else held.controller.abort();
      },
    };
  }

  get size(): number {
    return this.entries.size;
  }
}

export class LaneArtCache<TBitmap extends ArtBitmap = ImageBitmap> {
  private readonly bitmaps = new Map<string, TBitmap>();
  private readonly claims = new Map<string, LaneArtClaim<TBitmap>>();
  private readonly failed = new Map<string, string>();
  private readonly measured = new Map<string, readonly [number, number]>();
  private readonly trimmed = new Map<string, readonly [number, number]>();
  private readonly pending = new Map<string, LaneArtClaim<TBitmap>>();
  private readonly maxEntries: number;
  private readonly maxFailed: number;
  private readonly maxConcurrent: number;
  private readonly pool: LaneArtPool<TBitmap>;
  private onSettle?: () => void;
  private contentRevision: number;
  private settleVersion = 0;
  private failureRevision = 0;
  private visible: ReadonlySet<string> = new Set();
  private demanded: ReadonlySet<string> = new Set();
  private shownQueue: readonly string[] = [];
  private shownCursor = 0;
  private demandedFrom: Iterable<ArtResolution | string> | null = null;
  private demandedQueue: readonly string[] = [];
  private demandedCursor = 0;
  private disposed = false;

  constructor(options: LaneArtCacheOptions<TBitmap> = {}) {
    this.maxEntries = Math.max(1, options.maxEntries ?? DEFAULT_MAX_ENTRIES);
    this.maxFailed = Math.max(1, options.maxFailed ?? DEFAULT_MAX_FAILED);
    this.maxConcurrent = Math.max(1, options.maxConcurrent ?? DEFAULT_MAX_CONCURRENT);
    this.contentRevision = options.contentRevision ?? 0;
    this.pool = options.pool ?? new LaneArtPool(options.fetchBitmap);
    this.onSettle = options.onSettle;
  }

  setOnSettle(callback: (() => void) | undefined): void {
    this.onSettle = callback;
  }

  setContentRevision(revision: number): void {
    if (this.contentRevision === revision) return;
    this.contentRevision = revision;
    this.flush();
  }

  flush(): void {
    for (const claim of this.pending.values()) claim.release();
    this.pending.clear();
    this.releaseAll();
    this.measured.clear();
    this.trimmed.clear();
    if (this.failed.size > 0) {
      this.failed.clear();
      this.failureRevision++;
    }
    this.shownCursor = 0;
    this.demandedCursor = 0;
    this.settleVersion++;
    this.onSettle?.();
  }

  retry(asset?: string): void {
    if (this.disposed) return;
    if (asset !== undefined) {
      if (!this.failed.delete(asset)) return;
    } else {
      if (!this.failed.size) return;
      this.failed.clear();
    }
    this.failureRevision++;
    this.shownCursor = 0;
    this.demandedCursor = 0;
    this.pump();
    this.settleVersion++;
    this.onSettle?.();
  }

  resolve(resolution: ArtResolution): ArtResult<TBitmap> {
    switch (resolution.status) {
      case 'noArtExpected':
      case 'pending':
      case 'confirmedMissing':
      case 'unavailable':
      case 'refused':
        return resolution;
      case 'failed':
        return { ...resolution, retry: () => this.retry() };
      case 'ready': {
        const bitmap = this.touch(resolution.url);
        if (bitmap !== undefined) {
          return {
            ...resolution,
            bitmap,
          };
        }
        const failReason = this.failed.get(resolution.url);
        if (failReason !== undefined) {
          return {
            status: 'failed',
            reason: failReason,
            retryable: true,
            retry: () => this.retry(resolution.url),
          };
        }
        if (resolution.frame.outputWidth <= 0 || resolution.frame.outputHeight <= 0) {
          const dims = this.measured.get(resolution.url);
          if (dims !== undefined) {
            this.measured.delete(resolution.url);
            this.measured.set(resolution.url, dims);
            return {
              ...resolution,
              frame: {
                ...resolution.frame,
                outputWidth: dims[0],
                outputHeight: dims[1],
              },
            };
          }
        }
        return resolution;
      }
    }
  }

  resolveAddress(resolution: ArtResolution): ArtResult<never> {
    switch (resolution.status) {
      case 'noArtExpected':
      case 'pending':
      case 'confirmedMissing':
      case 'unavailable':
      case 'refused':
        return resolution;
      case 'failed':
        return { ...resolution, retry: () => this.retry() };
      case 'ready': {
        const failReason = this.failed.get(resolution.url);
        if (failReason !== undefined) {
          return {
            status: 'failed',
            reason: failReason,
            retryable: true,
            retry: () => this.retry(resolution.url),
          };
        }
        return resolution;
      }
    }
  }

  stateOfUrl(url: string): 'ready' | 'pending' | 'failed' | 'unrequested' {
    if (this.bitmaps.has(url)) return 'ready';
    if (this.pending.has(url)) return 'pending';
    if (this.failed.has(url)) return 'failed';
    return 'unrequested';
  }

  bitmapOf(url: string): TBitmap | undefined {
    return this.touch(url);
  }

  failureOf(url: string): string | undefined {
    return this.failed.get(url);
  }

  requestVisible(visible: Iterable<ArtResolution | string>, offscreen: Iterable<ArtResolution | string> = []): void {
    if (this.disposed) return;
    const urls = (items: Iterable<ArtResolution | string>, into: Set<string>) => {
      for (const item of items) {
        if (typeof item === 'string') {
          if (item) into.add(item);
        } else if (item.status === 'ready' && item.url) {
          into.add(item.url);
        }
      }
      return into;
    };
    const shown = urls(visible, new Set());
    this.visible = shown;
    this.shownQueue = [...shown];
    this.shownCursor = 0;
    if (offscreen !== this.demandedFrom) {
      const demanded = urls(offscreen, new Set());
      this.demandedFrom = offscreen;
      this.demanded = demanded;
      this.demandedQueue = [...demanded];
      this.demandedCursor = 0;
      for (const url of demanded) this.touch(url);
    }
    for (const [url, claim] of this.pending) {
      if (!this.wants(url)) {
        this.pending.delete(url);
        claim.release();
      }
    }
    for (const url of shown) this.touch(url);
    this.trim();
    this.pump();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.onSettle = undefined;
    for (const claim of this.pending.values()) claim.release();
    this.pending.clear();
    this.visible = new Set();
    this.demanded = new Set();
    this.demandedFrom = null;
    this.shownQueue = [];
    this.demandedQueue = [];
    this.releaseAll();
    this.measured.clear();
    this.trimmed.clear();
    this.failed.clear();
  }

  get cachedCount(): number {
    return this.bitmaps.size;
  }

  get pendingCount(): number {
    return this.pending.size;
  }

  get failedCount(): number {
    return this.failed.size;
  }

  get revision(): number {
    return this.contentRevision;
  }

  get version(): number {
    return this.settleVersion;
  }

  get failureVersion(): number {
    return this.failureRevision;
  }

  get map(): ReadonlyMap<string, TBitmap> {
    return this.bitmaps;
  }

  private touch(url: string): TBitmap | undefined {
    const held = this.bitmaps.get(url);
    if (held !== undefined) {
      this.bitmaps.delete(url);
      this.bitmaps.set(url, held);
    }
    return held;
  }

  private wants(url: string): boolean {
    return this.visible.has(url) || this.demanded.has(url);
  }

  private inactiveCount(): number {
    let count = 0;
    for (const url of this.bitmaps.keys()) if (!this.visible.has(url)) count++;
    for (const url of this.pending.keys()) if (!this.visible.has(url)) count++;
    return count;
  }

  private pump(): void {
    if (this.disposed) return;
    while (this.pending.size < this.maxConcurrent) {
      const shown = this.shownCursor < this.shownQueue.length;
      if (!shown && this.demandedCursor >= this.demandedQueue.length) return;
      const url = shown ? this.shownQueue[this.shownCursor]! : this.demandedQueue[this.demandedCursor]!;
      if (this.bitmaps.has(url) || this.pending.has(url) || this.failed.has(url)) {
        if (shown) this.shownCursor++;
        else this.demandedCursor++;
        continue;
      }
      if (!this.visible.has(url) && this.inactiveCount() >= this.maxEntries && !this.evictOne(key => !this.demanded.has(key))) return;
      if (shown) this.shownCursor++;
      else this.demandedCursor++;
      this.startFetch(url);
    }
  }

  private releaseAll(): void {
    for (const claim of this.claims.values()) claim.release();
    this.claims.clear();
    this.bitmaps.clear();
  }

  private drop(url: string): void {
    this.bitmaps.delete(url);
    this.claims.get(url)?.release();
    this.claims.delete(url);
  }

  private startFetch(url: string): void {
    const rev = this.contentRevision;
    const claim = this.pool.acquire(url, rev);
    this.pending.set(url, claim);
    void claim.bitmap.then(
      bitmap => {
        const current = this.pending.get(url) === claim;
        if (current) this.pending.delete(url);
        if (this.disposed || !current || this.contentRevision !== rev) {
          if (current) claim.release();
          return;
        }
        this.drop(url);
        if (bitmap.width > 0 && bitmap.height > 0) {
          this.rememberMeasured(url, bitmap.width, bitmap.height);
        }
        if (this.failed.delete(url)) this.failureRevision++;
        if (this.wants(url)) {
          this.bitmaps.set(url, bitmap);
          this.claims.set(url, claim);
          this.trim();
        } else {
          claim.release();
        }
        this.pump();
        this.settleVersion++;
        this.onSettle?.();
      },
      error => {
        const current = this.pending.get(url) === claim;
        if (current) {
          this.pending.delete(url);
          claim.release();
        }
        if (this.disposed || !current || this.contentRevision !== rev) return;
        const reason = error instanceof Error ? error.message : String(error || 'Art decode failed');
        this.failed.set(url, reason);
        while (this.failed.size > this.maxFailed) {
          const oldest = this.failed.keys().next().value;
          if (oldest === undefined) break;
          this.failed.delete(oldest);
        }
        this.failureRevision++;
        this.pump();
        this.settleVersion++;
        this.onSettle?.();
      },
    );
  }

  private evictOne(evictable: (url: string) => boolean): boolean {
    for (const key of this.bitmaps.keys()) {
      if (this.visible.has(key) || !evictable(key)) continue;
      this.drop(key);
      return true;
    }
    return false;
  }

  rememberTrimmed(url: string, width: number, height: number): void {
    if (width <= 0 || height <= 0) return;
    this.trimmed.delete(url);
    this.trimmed.set(url, [width, height]);
    while (this.trimmed.size > DEFAULT_MAX_MEASURED) {
      const oldest = this.trimmed.keys().next().value;
      if (oldest === undefined) break;
      this.trimmed.delete(oldest);
    }
  }

  trimmedOfUrl(url: string): readonly [number, number] | undefined {
    const dims = this.trimmed.get(url);
    if (dims !== undefined) {
      this.trimmed.delete(url);
      this.trimmed.set(url, dims);
    }
    return dims;
  }

  private rememberMeasured(url: string, width: number, height: number): void {
    this.measured.delete(url);
    this.measured.set(url, [width, height]);
    while (this.measured.size > DEFAULT_MAX_MEASURED) {
      const oldest = this.measured.keys().next().value;
      if (oldest === undefined) break;
      this.measured.delete(oldest);
    }
  }

  private trim(): void {
    let over = this.inactiveCount() - this.maxEntries;
    while (over > 0 && this.evictOne(key => !this.demanded.has(key))) over--;
    while (over > 0 && this.evictOne(() => true)) over--;
  }
}
