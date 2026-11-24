import type { ArtResolution, DecodedArtState } from './types.ts';

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
  status: 'pending' | 'ready' | 'failed';
  reason?: string;
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
      const existing = this.entries.get(asset);
      if (existing?.status === 'failed') this.entries.delete(asset);
      this.ensure(asset);
      this.wake(asset);
      return;
    }
    const urls = [...this.failed.keys()];
    this.failed.clear();
    for (const url of urls) {
      const existing = this.entries.get(url);
      if (existing?.status === 'failed') this.entries.delete(url);
      this.wake(url);
    }
  }

  resolve(resolution: ArtResolution): DecodedArtState<TImage> {
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
        const failReason = this.failed.get(resolution.asset);
        if (failReason !== undefined) {
          return {
            kind: 'failed',
            reason: failReason,
            retryable: true,
            retry: () => this.retry(resolution.asset),
          };
        }
        const entry = this.ensure(resolution.asset);
        if (!entry) return { kind: 'pending' };
        if (entry.status === 'ready') {
          return {
            kind: 'ready',
            bitmap: entry.image,
            origin: resolution.origin,
            ...(resolution.frame ? { frame: resolution.frame } : {}),
            width: resolution.width ?? entry.image.naturalWidth,
            height: resolution.height ?? entry.image.naturalHeight,
          };
        }
        if (entry.status === 'failed') {
          return {
            kind: 'failed',
            reason: entry.reason ?? 'Image decode failed',
            retryable: true,
            retry: () => this.retry(resolution.asset),
          };
        }
        return { kind: 'pending' };
      }
    }
  }

  readUrl(url: string, onSettle?: () => void): { readonly image: TImage | null; readonly pending: boolean; readonly failed: boolean } {
    const failReason = this.failed.get(url);
    if (failReason !== undefined) return { image: null, pending: false, failed: true };
    const entry = this.ensure(url);
    if (!entry) return { image: null, pending: false, failed: false };
    if (entry.status === 'ready') return { image: entry.image, pending: false, failed: false };
    if (entry.status === 'failed') return { image: null, pending: false, failed: true };
    if (onSettle) {
      let set = this.oneShotListeners.get(url);
      if (!set) this.oneShotListeners.set(url, set = new Set());
      set.add(onSettle);
    }
    return { image: null, pending: true, failed: false };
  }

  get cachedCount(): number {
    return this.entries.size;
  }

  get revision(): number {
    return this.contentRevision;
  }

  private ensure(url: string): Entry<TImage> | null {
    const existing = this.entries.get(url);
    if (existing) {
      this.entries.delete(url);
      this.entries.set(url, existing);
      if (existing.status === 'pending' && existing.image.complete) {
        if (existing.image.naturalWidth > 0) {
          existing.status = 'ready';
        } else {
          existing.status = 'failed';
          existing.reason = 'Image failed to load';
          this.recordFailure(url, existing.reason);
        }
      }
      return existing;
    }
    const create = this.createImage ?? (typeof Image !== 'undefined' ? (defaultCreateImage as unknown as () => TImage) : undefined);
    if (!create) return null;
    const rev = this.contentRevision;
    const image = create();
    const entry: Entry<TImage> = { image, status: 'pending' };
    this.entries.set(url, entry);
    image.onload = () => {
      if (this.contentRevision !== rev || this.entries.get(url) !== entry) return;
      entry.status = image.naturalWidth > 0 ? 'ready' : 'failed';
      if (entry.status === 'failed') {
        entry.reason = 'Image decoded empty';
        this.recordFailure(url, entry.reason);
      } else {
        this.failed.delete(url);
      }
      this.evict();
      this.wake(url);
    };
    image.onerror = () => {
      if (this.contentRevision !== rev || this.entries.get(url) !== entry) return;
      entry.status = 'failed';
      entry.reason = 'Image failed to load';
      this.recordFailure(url, entry.reason);
      this.wake(url);
    };
    image.src = url;
    if (image.complete && image.naturalWidth > 0) {
      entry.status = 'ready';
    }
    this.evict();
    return entry;
  }

  private recordFailure(url: string, reason: string): void {
    this.failed.set(url, reason);
    while (this.failed.size > this.maxFailed) {
      const oldest = this.failed.keys().next().value;
      if (oldest === undefined) break;
      this.failed.delete(oldest);
    }
  }

  private evict(): void {
    if (this.entries.size <= this.maxEntries) return;
    for (const [url, entry] of [...this.entries]) {
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
