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
  readonly onSettle?: () => void;
}

const DEFAULT_MAX_ENTRIES = 128;
const DEFAULT_MAX_FAILED = 256;
const DEFAULT_MAX_CONCURRENT = 6;

async function defaultFetchBitmap(url: string, signal: AbortSignal): Promise<ImageBitmap> {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error(`art ${response.status}`);
  const blob = await response.blob();
  if (signal.aborted) throw new DOMException('Aborted', 'AbortError');
  return createImageBitmap(blob);
}

export class LaneArtCache<TBitmap extends ArtBitmap = ImageBitmap> {
  private readonly bitmaps = new Map<string, TBitmap>();
  private readonly failed = new Map<string, string>();
  private readonly pending = new Map<string, AbortController>();
  private readonly maxEntries: number;
  private readonly maxFailed: number;
  private readonly maxConcurrent: number;
  private readonly fetchBitmap: LaneArtFetcher<TBitmap>;
  private onSettle?: () => void;
  private contentRevision: number;
  private settleVersion = 0;
  private visible: ReadonlySet<string> = new Set();
  private demanded: ReadonlySet<string> = new Set();
  private queue: readonly string[] = [];
  private cursor = 0;
  private disposed = false;

  constructor(options: LaneArtCacheOptions<TBitmap> = {}) {
    this.maxEntries = Math.max(1, options.maxEntries ?? DEFAULT_MAX_ENTRIES);
    this.maxFailed = Math.max(1, options.maxFailed ?? DEFAULT_MAX_FAILED);
    this.maxConcurrent = Math.max(1, options.maxConcurrent ?? DEFAULT_MAX_CONCURRENT);
    this.contentRevision = options.contentRevision ?? 0;
    this.fetchBitmap = options.fetchBitmap ?? (defaultFetchBitmap as LaneArtFetcher<TBitmap>);
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
    for (const controller of this.pending.values()) controller.abort();
    this.pending.clear();
    for (const bitmap of this.bitmaps.values()) bitmap.close();
    this.bitmaps.clear();
    this.failed.clear();
    this.cursor = 0;
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
    this.cursor = 0;
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
        return { status: 'pending' };
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
    const demanded = urls(offscreen, new Set());
    for (const url of shown) demanded.delete(url);
    this.visible = shown;
    this.demanded = demanded;
    this.queue = [...shown, ...demanded];
    this.cursor = 0;
    for (const [url, controller] of this.pending) {
      if (!this.wants(url)) {
        this.pending.delete(url);
        controller.abort();
      }
    }
    for (const url of this.queue) this.touch(url);
    this.evict();
    this.pump();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.onSettle = undefined;
    for (const controller of this.pending.values()) controller.abort();
    this.pending.clear();
    this.visible = new Set();
    this.demanded = new Set();
    this.queue = [];
    for (const bitmap of this.bitmaps.values()) bitmap.close();
    this.bitmaps.clear();
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

  private pump(): void {
    if (this.disposed) return;
    while (this.cursor < this.queue.length && this.pending.size < this.maxConcurrent) {
      const url = this.queue[this.cursor]!;
      this.cursor++;
      if (this.bitmaps.has(url) || this.pending.has(url) || this.failed.has(url)) continue;
      if (!this.visible.has(url)) {
        this.evict(this.maxEntries - this.pending.size - 1);
        if (this.bitmaps.size + this.pending.size >= this.maxEntries) return;
      }
      this.startFetch(url);
    }
  }

  private startFetch(url: string): void {
    const controller = new AbortController();
    const rev = this.contentRevision;
    this.pending.set(url, controller);
    void this.fetchBitmap(url, controller.signal).then(
      bitmap => {
        if (this.pending.get(url) === controller) this.pending.delete(url);
        if (this.disposed || controller.signal.aborted || this.contentRevision !== rev) {
          bitmap.close();
          return;
        }
        const prev = this.bitmaps.get(url);
        if (prev) {
          this.bitmaps.delete(url);
          prev.close();
        }
        this.failed.delete(url);
        if (this.wants(url)) {
          this.bitmaps.set(url, bitmap);
          this.evict();
        } else {
          bitmap.close();
        }
        this.pump();
        this.settleVersion++;
        this.onSettle?.();
      },
      error => {
        if (this.pending.get(url) === controller) this.pending.delete(url);
        if (this.disposed || controller.signal.aborted || this.contentRevision !== rev) return;
        const reason = error instanceof Error ? error.message : String(error || 'Art decode failed');
        this.failed.set(url, reason);
        while (this.failed.size > this.maxFailed) {
          const oldest = this.failed.keys().next().value;
          if (oldest === undefined) break;
          this.failed.delete(oldest);
        }
        this.pump();
        this.settleVersion++;
        this.onSettle?.();
      },
    );
  }

  private evict(limit = this.maxEntries): void {
    if (this.bitmaps.size <= limit) return;
    for (const [key, bitmap] of this.bitmaps) {
      if (this.bitmaps.size <= limit) break;
      if (this.wants(key)) continue;
      this.bitmaps.delete(key);
      bitmap.close();
    }
  }
}
