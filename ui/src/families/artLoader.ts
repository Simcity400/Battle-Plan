export interface FamilyArtBitmap {
  readonly width: number;
  readonly height: number;
  close(): void;
}

export type FamilyArtFetcher<TBitmap extends FamilyArtBitmap = ImageBitmap> = (
  url: string,
  signal: AbortSignal,
) => Promise<TBitmap>;

export interface FamilyArtLoaderOptions<TBitmap extends FamilyArtBitmap = ImageBitmap> {
  readonly maxEntries?: number;
  readonly maxFailed?: number;
  readonly fetchBitmap?: FamilyArtFetcher<TBitmap>;
  readonly onLoad?: () => void;
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

export class FamilyArtLoader<TBitmap extends FamilyArtBitmap = ImageBitmap> {
  private readonly bitmaps = new Map<string, TBitmap>();
  private readonly failed = new Set<string>();
  private readonly pending = new Map<string, AbortController>();
  private readonly maxEntries: number;
  private readonly maxFailed: number;
  private readonly fetchBitmap: FamilyArtFetcher<TBitmap>;
  private onLoad?: () => void;
  private disposed = false;

  constructor(options: FamilyArtLoaderOptions<TBitmap> = {}) {
    this.maxEntries = Math.max(1, options.maxEntries ?? DEFAULT_MAX_ENTRIES);
    this.maxFailed = Math.max(1, options.maxFailed ?? DEFAULT_MAX_FAILED);
    this.fetchBitmap = options.fetchBitmap ?? (defaultFetchBitmap as FamilyArtFetcher<TBitmap>);
    this.onLoad = options.onLoad;
  }

  setOnLoad(callback: (() => void) | undefined): void {
    this.onLoad = callback;
  }

  get(url: string): TBitmap | null | undefined {
    const held = this.bitmaps.get(url);
    if (held !== undefined) {
      this.bitmaps.delete(url);
      this.bitmaps.set(url, held);
      return held;
    }
    if (this.pending.has(url) || this.failed.has(url)) return null;
    return undefined;
  }

  requestVisible(urls: Iterable<string>): void {
    if (this.disposed) return;
    const wanted = new Set<string>();
    for (const url of urls) if (url) wanted.add(url);
    for (const [url, controller] of [...this.pending]) {
      if (!wanted.has(url)) {
        this.pending.delete(url);
        controller.abort();
      }
    }
    for (const url of wanted) {
      const existing = this.bitmaps.get(url);
      if (existing !== undefined) {
        this.bitmaps.delete(url);
        this.bitmaps.set(url, existing);
        continue;
      }
      if (this.pending.has(url) || this.failed.has(url)) continue;
      const controller = new AbortController();
      this.pending.set(url, controller);
      void this.fetchBitmap(url, controller.signal).then(
        bitmap => {
          if (this.pending.get(url) === controller) this.pending.delete(url);
          if (this.disposed || controller.signal.aborted) {
            bitmap.close();
            return;
          }
          const prev = this.bitmaps.get(url);
          if (prev) {
            this.bitmaps.delete(url);
            prev.close();
          }
          this.bitmaps.set(url, bitmap);
          this.evict(wanted);
          this.onLoad?.();
        },
        () => {
          if (this.pending.get(url) === controller) this.pending.delete(url);
          if (this.disposed || controller.signal.aborted) return;
          this.failed.add(url);
          while (this.failed.size > this.maxFailed) {
            this.failed.delete(this.failed.values().next().value!);
          }
        },
      );
    }
  }

  private evict(protectedUrls?: ReadonlySet<string>): void {
    if (this.bitmaps.size <= this.maxEntries) return;
    for (const [key, bitmap] of [...this.bitmaps]) {
      if (this.bitmaps.size <= this.maxEntries) break;
      if (protectedUrls?.has(key)) continue;
      this.bitmaps.delete(key);
      bitmap.close();
    }
    while (this.bitmaps.size > this.maxEntries) {
      const oldestKey = this.bitmaps.keys().next().value!;
      const oldest = this.bitmaps.get(oldestKey)!;
      this.bitmaps.delete(oldestKey);
      oldest.close();
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.onLoad = undefined;
    for (const controller of this.pending.values()) controller.abort();
    this.pending.clear();
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

  get map(): ReadonlyMap<string, TBitmap | null> {
    return this.bitmaps;
  }
}
