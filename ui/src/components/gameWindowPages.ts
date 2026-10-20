import type { WindowTextureImage } from '../generated/dashboard';

/** The bytes of page images the app keeps across windows before it lets the least recently
 * used released sessions go. */
export const PAGE_BUDGET = 64 * 1024 * 1024;

interface Held {
  pages: Map<number, WindowTextureImage>;
  /** Pages multiplied by a colour, by page and colour, as object URLs. */
  tinted: Map<string, string>;
  bytes: number;
  /** Windows drawing from it now. */
  users: number;
  used: number;
}

/** Texture pages by owner (a client and an artwork session), so scrolls, hours and reopened
 * windows reuse them. A session no window draws from is released, kept while the budget
 * allows, and the least recently used released ones go first; one in use is never dropped. */
export class PageCache {
  private held = new Map<string, Held>();
  private clock = 0;
  private total = 0;
  private revisions = new Map<string, number>();
  private listeners = new Map<string, Set<() => void>>();

  private readonly budget: number;
  private readonly revoke: (url: string) => void;

  constructor(budget = PAGE_BUDGET, revoke: (url: string) => void = url => URL.revokeObjectURL(url)) {
    this.budget = budget;
    this.revoke = revoke;
  }

  private entry(key: string): Held {
    let held = this.held.get(key);
    if (!held) this.held.set(key, held = { pages: new Map(), tinted: new Map(), bytes: 0, users: 0, used: 0 });
    held.used = ++this.clock;
    return held;
  }

  /** A window follows the pages of its own owner `key`: any page added to it, coloured or let
   * go changes `key`'s revision and tells that owner's windows alone, so a window whose page
   * another window of the same owner loaded or coloured draws it too, and no other redraws. */
  subscribe(key: string, listener: () => void): () => void {
    let listeners = this.listeners.get(key);
    if (!listeners) this.listeners.set(key, listeners = new Set());
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      if (!listeners.size && this.listeners.get(key) === listeners) this.listeners.delete(key);
      this.retire(key);
    };
  }

  /** Forgets `key`'s revision once it holds no pages and no window follows it. */
  private retire(key: string) {
    if (!this.held.has(key) && !this.listeners.has(key)) this.revisions.delete(key);
  }

  /** The owners whose revision is kept, for tests of its bound. */
  trackedOwners(): number { return this.revisions.size; }

  revision(key: string): number { return this.revisions.get(key) ?? 0; }

  private changed(key: string) {
    this.revisions.set(key, this.revision(key) + 1);
    for (const listener of this.listeners.get(key) ?? []) listener();
  }

  /** A window starts drawing from `key`'s pages. */
  acquire(key: string): void { this.entry(key).users++; }

  /** A window stops drawing from `key`'s pages. */
  release(key: string): void {
    const held = this.held.get(key);
    if (!held) return;
    held.users = Math.max(0, held.users - 1);
    this.evict();
  }

  pages(key: string): ReadonlyMap<number, WindowTextureImage> { return this.held.get(key)?.pages ?? NO_PAGES; }
  tinted(key: string): ReadonlyMap<string, string> { return this.held.get(key)?.tinted ?? NO_TINTS; }

  add(key: string, page: WindowTextureImage): void {
    const held = this.entry(key);
    this.charge(held, (page.image?.length ?? 0) - (held.pages.get(page.id)?.image?.length ?? 0));
    held.pages.set(page.id, page);
    this.evict();
    this.changed(key);
  }

  /** Keeps a page multiplied by a colour; false (and the URL revoked) when `key` was let go. */
  addTinted(key: string, name: string, url: string, bytes: number): boolean {
    const held = this.held.get(key);
    if (!held || held.tinted.has(name)) { this.revoke(url); return false; }
    held.tinted.set(name, url);
    held.used = ++this.clock;
    this.charge(held, bytes);
    this.evict();
    this.changed(key);
    return true;
  }

  /** The bytes kept, for the budget. */
  bytes(): number { return this.total; }
  has(key: string): boolean { return this.held.has(key); }

  private charge(held: Held, bytes: number) { held.bytes += bytes; this.total += bytes; }

  private evict() {
    while (this.total > this.budget) {
      let oldest: [string, Held] | null = null;
      for (const entry of this.held) if (entry[1].users === 0 && (!oldest || entry[1].used < oldest[1].used)) oldest = entry;
      if (!oldest) return;
      const [key, held] = oldest;
      for (const url of held.tinted.values()) this.revoke(url);
      this.total -= held.bytes;
      this.held.delete(key);
      // Its windows hear it went; with none, its revision goes too.
      this.changed(key);
      this.retire(key);
    }
  }
}

const NO_PAGES: ReadonlyMap<number, WindowTextureImage> = new Map();
const NO_TINTS: ReadonlyMap<string, string> = new Map();

export const pageCache = new PageCache();
