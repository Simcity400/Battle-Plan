import type { ArtBitmap, ArtResolution, DecodedArtState } from './types.ts';

export type LaneArtFetcher<TBitmap extends ArtBitmap = ImageBitmap> = (
  url: string,
  signal: AbortSignal,
) => Promise<TBitmap>;

export interface LaneArtCacheOptions<TBitmap extends ArtBitmap = ImageBitmap> {
  readonly maxEntries?: number;
  readonly maxFailed?: number;
  readonly contentRevision?: number;
  readonly fetchBitmap?: LaneArtFetcher<TBitmap>;
  readonly onSettle?: () => void;
}

const DEFAULT_MAX_ENTRIES = 128;
const DEFAULT_MAX_FAILED = 256;

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
  private readonly fetchBitmap: LaneArtFetcher<TBitmap>;
  private onSettle?: () => void;
  private contentRevision: number;
  private wanted: ReadonlySet<string> = new Set();
  private disposed = false;

  constructor(options: LaneArtCacheOptions<TBitmap> = {}) {
    this.maxEntries = Math.max(1, options.maxEntries ?? DEFAULT_MAX_ENTRIES);
    this.maxFailed = Math.max(1, options.maxFailed ?? DEFAULT_MAX_FAILED);
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
    this.onSettle?.();
  }

  retry(asset?: string): void {
    if (this.disposed) return;
    if (asset !== undefined) {
      if (!this.failed.delete(asset)) return;
      if (this.wanted.has(asset)) this.startFetch(asset);
      this.onSettle?.();
      return;
    }
    if (!this.failed.size) return;
    const retryable = [...this.failed.keys()];
    this.failed.clear();
    for (const url of retryable) {
      if (this.wanted.has(url)) this.startFetch(url);
    }
    this.onSettle?.();
  }

  resolve(resolution: ArtResolution): DecodedArtState<TBitmap> {
    switch (resolution.kind) {
      case 'none':
        return { kind: 'none' };
      case 'pending':
        return { kind: 'pending' };
      case 'missing':
        return { kind: 'missing', reason: resolution.reason, ...(resolution.origin ? { origin: resolution.origin } : {}) };
      case 'refused':
        return { kind: 'refused', reason: resolution.reason };
      case 'failed':
        return {
          kind: 'failed',
          reason: resolution.reason,
          retryable: resolution.retryable,
          retry: () => this.retry(),
        };
      case 'ready': {
        const bitmap = this.touch(resolution.asset);
        if (bitmap !== undefined) {
          return {
            kind: 'ready',
            bitmap,
            origin: resolution.origin,
            ...(resolution.frame ? { frame: resolution.frame } : {}),
            width: resolution.width ?? bitmap.width,
            height: resolution.height ?? bitmap.height,
          };
        }
        const failReason = this.failed.get(resolution.asset);
        if (failReason !== undefined) {
          return {
            kind: 'failed',
            reason: failReason,
            retryable: true,
            retry: () => this.retry(resolution.asset),
          };
        }
        return { kind: 'pending' };
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

  requestVisible(resolutions: Iterable<ArtResolution | string>): void {
    if (this.disposed) return;
    const wanted = new Set<string>();
    for (const item of resolutions) {
      if (typeof item === 'string') {
        if (item) wanted.add(item);
      } else if (item.kind === 'ready' && item.asset) {
        wanted.add(item.asset);
      }
    }
    this.wanted = wanted;
    for (const [url, controller] of [...this.pending]) {
      if (!wanted.has(url)) {
        this.pending.delete(url);
        controller.abort();
      }
    }
    for (const url of wanted) {
      if (this.touch(url) !== undefined) continue;
      if (this.pending.has(url) || this.failed.has(url)) continue;
      this.startFetch(url);
    }
    this.evict();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.onSettle = undefined;
    for (const controller of this.pending.values()) controller.abort();
    this.pending.clear();
    this.wanted = new Set();
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
        this.bitmaps.set(url, bitmap);
        this.evict();
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
        this.onSettle?.();
      },
    );
  }

  private evict(): void {
    if (this.bitmaps.size <= this.maxEntries) return;
    for (const [key, bitmap] of [...this.bitmaps]) {
      if (this.bitmaps.size <= this.maxEntries) break;
      if (this.wanted.has(key)) continue;
      this.bitmaps.delete(key);
      bitmap.close();
    }
  }
}
