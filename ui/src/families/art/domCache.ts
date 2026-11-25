import type { ArtResolution, ArtResult } from './types.ts';

export interface DomArtImage {
  readonly complete: boolean;
  readonly naturalWidth: number;
  readonly naturalHeight: number;
  src: string;
  onload: ((...args: any[]) => unknown) | null;
  onerror: ((...args: any[]) => unknown) | null;
}

export interface DomArtCacheOptions<TImage extends DomArtImage = HTMLImageElement> {
  readonly maxEntries?: number;
  readonly maxFailed?: number;
  readonly createImage?: () => TImage;
}

interface Entry<TImage extends DomArtImage> {
  readonly image: TImage;
  status: 'pending' | 'ready';
}

const DEFAULT_MAX_ENTRIES = 2048;
const DEFAULT_MAX_FAILED = 2048;

function defaultCreateImage(): HTMLImageElement {
  return new Image();
}

export class DomArtCache<TImage extends DomArtImage = HTMLImageElement> {
  private readonly entries = new Map<string, Entry<TImage>>();
  private readonly failed = new Map<string, string>();
  private readonly listeners = new Map<string, Set<() => void>>();
  private readonly oneShotListeners = new Map<string, Set<() => void>>();
  private readonly globalListeners = new Set<() => void>();
  private readonly maxEntries: number;
  private readonly maxFailed: number;
  private readonly createImage?: () => TImage;
  private contentRevision = 0;

  constructor(options: DomArtCacheOptions<TImage> = {}) {
    this.maxEntries = Math.max(1, options.maxEntries ?? DEFAULT_MAX_ENTRIES);
    this.maxFailed = Math.max(1, options.maxFailed ?? DEFAULT_MAX_FAILED);
    this.createImage = options.createImage;
  }

  setContentRevision(revision: number): void {
    if (this.contentRevision === revision) return;
    this.contentRevision = revision;
    this.flush();
  }

  flush(): void {
    for (const entry of this.entries.values()) {
      entry.image.onload = null;
      entry.image.onerror = null;
    }
    this.entries.clear();
    this.failed.clear();
    this.oneShotListeners.clear();
    for (const set of [...this.listeners.values()]) for (const fn of [...set]) fn();
    for (const fn of [...this.globalListeners]) fn();
  }

  subscribe(asset: string, listener: () => void): () => void {
    let set = this.listeners.get(asset);
    if (!set) this.listeners.set(asset, set = new Set());
    set.add(listener);
    return () => {
      set.delete(listener);
      if (!set.size && this.listeners.get(asset) === set) this.listeners.delete(asset);
    };
  }

  subscribeAll(listener: () => void): () => void {
    this.globalListeners.add(listener);
    return () => { this.globalListeners.delete(listener); };
  }

  retry(asset?: string): void {
    if (asset !== undefined) {
      this.failed.delete(asset);
      this.ensure(asset);
      this.wake(asset);
      return;
    }
    const urls = [...this.failed.keys()];
    this.failed.clear();
    for (const url of urls) this.wake(url);
  }

  resolve(resolution: ArtResolution): ArtResult<TImage> {
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
        const entry = this.failed.has(resolution.url) ? null : this.ensure(resolution.url);
        const failReason = this.failed.get(resolution.url);
        if (failReason !== undefined) {
          return {
            status: 'failed',
            reason: failReason,
            retryable: true,
            retry: () => this.retry(resolution.url),
          };
        }
        if (entry?.status === 'ready') {
          return {
            ...resolution,
            bitmap: entry.image,
          };
        }
        return { status: 'pending' };
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
        const entry = this.failed.has(resolution.url) ? null : this.ensure(resolution.url);
        const failReason = this.failed.get(resolution.url);
        if (failReason !== undefined) {
          return {
            status: 'failed',
            reason: failReason,
            retryable: true,
            retry: () => this.retry(resolution.url),
          };
        }
        return entry?.status === 'ready' ? resolution : { status: 'pending' };
      }
    }
  }

  readUrl(url: string, onSettle?: () => void): { readonly image: TImage | null; readonly pending: boolean; readonly failed: boolean } {
    const entry = this.failed.has(url) ? null : this.ensure(url);
    if (this.failed.has(url)) return { image: null, pending: false, failed: true };
    if (!entry) return { image: null, pending: false, failed: false };
    if (entry.status === 'ready') return { image: entry.image, pending: false, failed: false };
    if (onSettle) {
      let set = this.oneShotListeners.get(url);
      if (!set) this.oneShotListeners.set(url, set = new Set());
      set.add(onSettle);
    }
    return { image: null, pending: true, failed: false };
  }

  stateOfUrl(url: string): 'ready' | 'pending' | 'failed' | 'unrequested' {
    if (this.failed.has(url)) return 'failed';
    const entry = this.entries.get(url);
    if (!entry) return 'unrequested';
    this.settleComplete(url, entry);
    return this.failed.has(url) ? 'failed' : entry.status;
  }

  get cachedCount(): number {
    return this.entries.size;
  }

  get failedCount(): number {
    return this.failed.size;
  }

  get revision(): number {
    return this.contentRevision;
  }

  private ensure(url: string): Entry<TImage> | null {
    const existing = this.entries.get(url);
    if (existing) {
      this.entries.delete(url);
      this.entries.set(url, existing);
      this.settleComplete(url, existing);
      return this.entries.get(url) === existing ? existing : null;
    }
    const create = this.createImage ?? (typeof Image !== 'undefined' ? (defaultCreateImage as unknown as () => TImage) : undefined);
    if (!create) return null;
    const rev = this.contentRevision;
    const image = create();
    const entry: Entry<TImage> = { image, status: 'pending' };
    this.entries.set(url, entry);
    image.onload = () => {
      if (this.contentRevision !== rev || this.entries.get(url) !== entry) return;
      if (image.naturalWidth > 0) {
        entry.status = 'ready';
        this.failed.delete(url);
        this.evict();
      } else {
        this.fail(url, entry, 'Image decoded empty');
      }
      this.wake(url);
    };
    image.onerror = () => {
      if (this.contentRevision !== rev || this.entries.get(url) !== entry) return;
      this.fail(url, entry, 'Image failed to load');
      this.wake(url);
    };
    image.src = url;
    if (image.complete && image.naturalWidth > 0) {
      entry.status = 'ready';
    }
    this.evict();
    return entry;
  }

  private settleComplete(url: string, entry: Entry<TImage>): void {
    if (entry.status !== 'pending' || !entry.image.complete) return;
    if (entry.image.naturalWidth > 0) entry.status = 'ready';
    else this.fail(url, entry, 'Image failed to load');
  }

  private fail(url: string, entry: Entry<TImage>, reason: string): void {
    entry.image.onload = null;
    entry.image.onerror = null;
    if (this.entries.get(url) === entry) this.entries.delete(url);
    this.failed.delete(url);
    this.failed.set(url, reason);
    while (this.failed.size > this.maxFailed) {
      const oldest = this.failed.keys().next().value;
      if (oldest === undefined) break;
      this.failed.delete(oldest);
    }
  }

  private evict(): void {
    if (this.entries.size <= this.maxEntries) return;
    for (const [url, entry] of this.entries) {
      if (this.entries.size <= this.maxEntries) break;
      if (entry.status === 'pending' || this.listeners.has(url)) continue;
      entry.image.onload = null;
      entry.image.onerror = null;
      this.entries.delete(url);
    }
  }

  private wake(url: string): void {
    const set = this.listeners.get(url);
    if (set) for (const fn of [...set]) fn();
    const oneshots = this.oneShotListeners.get(url);
    if (oneshots) {
      this.oneShotListeners.delete(url);
      for (const fn of [...oneshots]) fn();
    }
    for (const fn of [...this.globalListeners]) fn();
  }
}

export const sharedDomArtCache = new DomArtCache<HTMLImageElement>();
