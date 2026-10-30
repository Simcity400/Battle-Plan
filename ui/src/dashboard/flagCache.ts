import type { DashboardClient, ProductRequest } from './client';
import { validateProductReply } from './client.ts';
import { contentRevision } from './contentRevision.ts';
import type { CountryFlagImage, CountryFlagParams, CountryFlagReply } from '../generated/dashboard';

/** A list flag as the picker keeps it: its picture as the PNG data URL the reply carries. */
export interface StoredFlag {
  tag: string;
  asset: string;
  width: number;
  height: number;
  png: string;
}

/** Bytes for the flags the picker keeps, close to the product cache's own budget. */
export const FLAG_STORE_BYTES = 64 * 1024 * 1024;
/** Map entry, key object and flag header bookkeeping, approximately. */
const ENTRY_OVERHEAD_BYTES = 96;

function storedBytes(key: string, value: StoredFlag | null): number {
  const strings = key.length + (value ? value.tag.length + value.asset.length + value.png.length : 0);
  return ENTRY_OVERHEAD_BYTES + strings * 2;
}

export function storedFlag(image: CountryFlagImage): StoredFlag {
  return { tag: image.tag, asset: image.asset, width: image.width, height: image.height, png: image.png };
}

/** One list flag per country and observation hour, bounded by bytes. Kept apart
 * from the product cache so a long country list never evicts widget data. */
export class FlagStore {
  private entries = new Map<string, { value: StoredFlag | null; bytes: number }>();
  private maxBytes: number;
  private totalBytes = 0;
  constructor(maxBytes: number) { this.maxBytes = maxBytes; }

  static key(sourceId: string, revision: string, branchId: string, hour: number, tag: string): string {
    return JSON.stringify([sourceId, revision, branchId, hour, tag]);
  }

  /** A flag, null when the recording has no flag for it, or undefined when not loaded. */
  get(key: string): StoredFlag | null | undefined {
    const entry = this.entries.get(key);
    if (entry === undefined) return undefined;
    this.entries.delete(key); this.entries.set(key, entry);
    return entry.value;
  }

  set(key: string, value: StoredFlag | null): void {
    this.delete(key);
    const bytes = storedBytes(key, value);
    this.entries.set(key, { value, bytes });
    this.totalBytes += bytes;
    // Least recently used first; the newest entry always stays.
    for (const oldest of this.entries.keys()) {
      if (this.totalBytes <= this.maxBytes || oldest === key) break;
      this.delete(oldest);
    }
  }

  get size(): number { return this.entries.size; }
  get bytes(): number { return this.totalBytes; }

  deleteSource(sourceId: string): void {
    for (const key of [...this.entries.keys()]) if (JSON.parse(key)[0] === sourceId) this.delete(key);
  }

  private delete(key: string): void {
    const entry = this.entries.get(key);
    if (!entry) return;
    this.entries.delete(key);
    this.totalBytes -= entry.bytes;
  }
}

/** Flags requested per product call; the product accepts at most this many tags. */
export const FLAG_BATCH = 32;
const stores = new WeakMap<DashboardClient, { content: number; store: FlagStore }>();

export function flagStore(client: DashboardClient): FlagStore {
  const content = contentRevision(client).snapshot();
  let entry = stores.get(client);
  if (!entry || entry.content !== content) { entry = { content, store: new FlagStore(FLAG_STORE_BYTES) }; stores.set(client, entry); }
  return entry.store;
}

export function forgetFlags(client: DashboardClient, sourceId: string): void {
  stores.get(client)?.store.deleteSource(sourceId);
}

export interface FlagTarget { tag: string; branchId: string; hour: number }

/** Load the list shield flags not yet stored, grouped by observation hour and batched
 * within the product limit. Returns the replies' warnings, which describe the
 * recording (for example its content coverage) rather than a failed flag. */
export async function loadListFlags(client: DashboardClient, source: { id: string; revision: string },
  targets: readonly FlagTarget[], signal: AbortSignal): Promise<string[]> {
  const store = flagStore(client);
  const groups = new Map<string, FlagTarget[]>();
  for (const target of targets) {
    if (store.get(FlagStore.key(source.id, source.revision, target.branchId, target.hour, target.tag)) !== undefined) continue;
    const key = `${target.branchId}:${target.hour}`;
    const group = groups.get(key) ?? [];
    group.push(target); groups.set(key, group);
  }
  const warnings: string[] = [];
  for (const group of groups.values()) for (let start = 0; start < group.length; start += FLAG_BATCH) {
    const batch = group.slice(start, start + FLAG_BATCH);
    const { branchId, hour } = batch[0];
    // `panel` is app styling: the native top-bar shield with its overlay's edge shading evened out.
    const request: ProductRequest<CountryFlagParams> = { sourceId: source.id, revision: source.revision, product: 'country-flags',
      params: { branchId, hour, tags: batch.map(target => target.tag), size: 'panel' } };
    const reply = validateProductReply(await client.product<CountryFlagReply>(request, signal), request).data;
    if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
    const images = new Map(reply.flags.map(image => [image.tag, image]));
    for (const target of batch) {
      const image = images.get(target.tag);
      store.set(FlagStore.key(source.id, source.revision, branchId, hour, target.tag), image ? storedFlag(image) : null);
    }
    warnings.push(...reply.warnings);
  }
  return warnings;
}
