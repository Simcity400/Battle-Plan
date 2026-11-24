import type { DashboardClient, DashboardSource, ProductReply, ProductRequest } from '../dashboard/client';
import { queryFailure } from '../dashboard/queryError.ts';
import type { QueryError, SourceReply } from '../generated/dashboard';
import { packet, resetStamps, splitForLanes } from '../dashboard/follow/split.ts';
import { upgradeRecorded } from './recordedWire.ts';
import { DATED_FOLLOW, DATED_PANEL } from '../dashboard/follow/message.ts';
import { parseDatedPanels } from './followMessage.ts';
import { isCellsId } from '../dashboard/follow/liveCells.ts';
import { busy, busyPreparing } from '../shell/busy.ts';

export const RECORDED_KIND = 'battleplan recorded replies';
export const RECORDED_VERSION = 4;

export interface RecordedHeader { kind: string; version: number; formatDigest: string; format: string[]; app: { revision: string }; recordedAt: string }
export interface RecordedBox { x: number; y: number; width: number; height: number }
export interface RecordedLayout {
  viewport: { width: number; height: number; dpr: number };
  grid: RecordedBox | null;
  scroll: RecordedBox | null;
  widgets: (RecordedBox & { id: string; kind: string | null })[];
}
interface Query { sourceId: string; revision: string; product: string; params: unknown }
export type RecordedEntry =
  | { t: 'open'; at: number; head: number | null; ok: boolean; reply: unknown }
  | { t: 'product'; at: number; ms: number; head: number | null; args: { query: Query } | null; ok: boolean; reply: unknown }
  | { t: 'follow'; at: number; head: number | null; bytes: string }
  | { t: 'mark'; at: number; head: number | null; label: string; page?: string; speed?: number; trace?: boolean; layout?: RecordedLayout };
export type RecordedMark = Extract<RecordedEntry, { t: 'mark' }>;
type FollowEntry = Extract<RecordedEntry, { t: 'follow' }>;
type ReplyEntry = Extract<RecordedEntry, { t: 'product' }>;
export interface RecordedFormat { file: string | null; formatFile: string; digest: string; lines: string[] }

export function recordedRefusal(file: string, header: Partial<RecordedHeader>, tree: RecordedFormat): string | null {
  if (header.kind !== RECORDED_KIND || header.version !== RECORDED_VERSION)
    return `${file} is not recorded replies of version ${RECORDED_VERSION} (it says ${JSON.stringify(header.kind ?? null)} version ${JSON.stringify(header.version ?? null)})`;
  if (header.formatDigest === tree.digest) return null;
  const table = (line: string) => line.split(' ').slice(0, 2).join(' ');
  const was = new Map((header.format ?? []).map(line => [table(line), line])), now = new Map(tree.lines.map(line => [table(line), line]));
  const added = [...now.keys()].filter(name => !was.has(name)), removed = [...was.keys()].filter(name => !now.has(name));
  const changed = [...now.keys()].filter(name => was.has(name) && was.get(name) !== now.get(name));
  const named = (label: string, names: string[]) => names.length ? [`${label} ${names.slice(0, 8).join(', ')}${names.length > 8 ? ` and ${names.length - 8} more` : ''}`] : [];
  const differences = [...named('added', added), ...named('removed', removed), ...named('changed', changed)];
  return `${file} was recorded by app ${header.app?.revision?.slice(0, 10) ?? 'unknown'} with another recording format than this tree's `
    + `(${tree.formatFile}: ${differences.join('; ') || 'the same tables in another order'}): record the replies again from the current format's 13-year recording`;
}

export async function loadRecorded(url = '/recorded-replies'): Promise<Recorded> {
  const formatReply = await fetch(`${url}/format`);
  if (!formatReply.ok) throw new Error(await formatReply.text());
  const tree = await formatReply.json() as RecordedFormat;
  const body = await fetch(url);
  if (!body.ok || !body.body) throw new Error(await body.text());
  const entries: RecordedEntry[] = [];
  let header: RecordedHeader | null = null, rest = '';
  const reader = body.body.pipeThrough(new DecompressionStream('gzip')).pipeThrough(new TextDecoderStream()).getReader();
  for (;;) {
    const { done, value } = await reader.read();
    const text = rest + (value ?? '');
    const lines = text.split('\n');
    rest = done ? '' : lines.pop()!;
    for (const line of lines) {
      if (!line) continue;
      if (header) { entries.push(JSON.parse(line) as RecordedEntry); continue; }
      header = JSON.parse(line) as RecordedHeader;
      const refused = recordedRefusal(tree.file ?? url, header, tree);
      if (refused) { void reader.cancel(); throw new Error(refused); }
    }
    if (done) break;
  }
  if (!header) throw new Error(`${tree.file ?? url} holds no recorded replies`);
  return new Recorded(tree.file ?? url, header, entries);
}

function bytesOf(base64: string): Uint8Array {
  const text = atob(base64), bytes = new Uint8Array(text.length);
  for (let at = 0; at < text.length; at++) bytes[at] = text.charCodeAt(at);
  return bytes;
}

export function messageHour(bytes: Uint8Array): number | null {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 4) return null;
  const magic = view.getUint32(0, true);
  if (magic === DATED_FOLLOW && bytes.byteLength >= 24) return view.getInt32(8, true);
  if (magic !== DATED_PANEL) return null;
  const { panels, completed } = parseDatedPanels(bytes);
  const hours = [...panels.map(panel => panel.hour), ...completed === undefined ? [] : [completed]];
  return hours.length ? Math.max(...hours) : null;
}

export function remapMessage(bytes: Uint8Array, idOf: (recorded: number) => number | undefined): Uint8Array | null {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 4) return bytes;
  const magic = view.getUint32(0, true);
  const mapped = (id: number) => isCellsId(id) ? id : idOf(id);
  const panel = magic === DATED_PANEL;
  if (!panel && magic !== DATED_FOLLOW) return bytes;
  const countAt = panel ? 8 : 12;
  const kept: [number, number, number][] = [];
  for (let at = 24, count = view.getUint32(countAt, true), answer = 0; answer < count; answer++) {
    const end = panel ? at + ((16 + view.getUint32(at + 12, true) + 7) & ~7) : at + 16 + view.getUint32(at + 8, true);
    const recorded = view.getUint32(at, true);
    const id = panel && recorded === 0 ? 0 : mapped(recorded);
    if (id !== undefined) kept.push([at, end, id]);
    at = end;
  }
  if (!kept.length && (panel || view.getUint32(countAt, true))) return null;
  const out = new Uint8Array(24 + kept.reduce((sum, [start, end]) => sum + end - start, 0));
  const written = new DataView(out.buffer);
  out.set(bytes.subarray(0, 24));
  written.setUint32(countAt, kept.length, true);
  let at = 24;
  for (const [start, end, id] of kept) {
    out.set(bytes.subarray(start, end), at);
    written.setUint32(at, id, true);
    at += end - start;
  }
  return out;
}

const stableJson = (value: unknown): string => JSON.stringify(value, (_key, item: unknown) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) : item);
const ITEMIZED: Record<string, { asks: string; answers: string[]; place?: string; item?: string }> = {
  'country-flags': { asks: 'flags', answers: ['flags', 'unavailable'], place: 'flag' },
  'unit-leader-portraits': { asks: 'references', answers: ['portraits'], item: 'reference' },
};
type Itemized = { reply: { data: Record<string, unknown> }; at: number; item: unknown };

const productKey = (product: string, params: unknown) => `${product}\u0000${stableJson(params)}`;
type FollowParams = { id?: number; branchId: string; clock: number; query?: unknown; playing?: boolean };
const followKey = (params: unknown) => { const { branchId, clock, query } = params as FollowParams; return stableJson({ branchId, clock, query }); };
const clockKey = (params: unknown) => { const { branchId, clock } = params as FollowParams; return stableJson({ branchId, clock }); };

export interface RecordedAsk { product: string; params: unknown; asksFrom?: number; served: boolean; at: number; lane: boolean; head: number | null }

export class RecordedClient implements DashboardClient {
  readonly available = true;
  readonly asked: RecordedAsk[] = [];
  window: readonly Uint8Array[] = [];
  at = 0;
  windowEnd = 0;
  hourOf: () => number | null = () => null;
  private readonly replies = new Map<string, ReplyEntry>();
  private readonly items = new Map<string, Itemized>();
  private readonly follows = new Map<string, { ids: Set<number>; entry: ReplyEntry }>();
  private readonly anchors = new Map<string, ReplyEntry[]>();
  private unfollow?: ReplyEntry;
  private readonly ids = new Map<number, number>();
  private ports: readonly MessagePort[] = [];

  constructor(entries: readonly RecordedEntry[]) {
    for (const entry of entries) {
      if (entry.t !== 'product' || !entry.args) continue;
      const { product, params } = entry.args.query;
      if (product === 'follow') {
        const key = followKey(params), id = (params as { id: number }).id;
        const held = this.follows.get(key);
        if (held) held.ids.add(id); else this.follows.set(key, { ids: new Set([id]), entry });
      } else if (product === 'follow-clock') {
        const key = clockKey(params);
        let anchors = this.anchors.get(key);
        if (!anchors) this.anchors.set(key, anchors = []);
        anchors.push(entry);
      } else if (product === 'unfollow') this.unfollow = entry;
      else if (ITEMIZED[product]) {
        if (!entry.ok) continue;
        const { asks } = ITEMIZED[product];
        const { [asks]: listed, ...rest } = params as Record<string, unknown>;
        (listed as unknown[]).forEach((item, at) => {
          const key = productKey(product, { ...rest, [asks]: item });
          if (!this.items.has(key)) this.items.set(key, { reply: entry.reply as { data: Record<string, unknown> }, at, item });
        });
      } else {
        const key = productKey(product, params), held = this.replies.get(key);
        if (!held || (!held.ok && entry.ok)) this.replies.set(key, entry);
      }
    }
    for (const entries of this.anchors.values()) entries.sort((a, b) => a.at - b.at);
  }

  get misses(): RecordedAsk[] { return this.asked.filter(ask => !ask.served); }

  async product<T>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>> {
    const before = busyPreparing(), done = busy('requests');
    try { return await this.answer<T>(request, signal, busyPreparing() > before); } finally { done(); }
  }

  private async answer<T>(request: ProductRequest, signal: AbortSignal, lane: boolean): Promise<ProductReply<T>> {
    const { product, params, asksFrom } = request;
    let entry: ReplyEntry | undefined;
    if (product === 'follow') {
      const held = this.follows.get(followKey(params));
      if (held?.entry.ok && (held.entry.reply as ProductReply<{ available: boolean }>).data.available) {
        for (const id of held.ids) this.ids.set(id, (params as { id: number }).id);
        setTimeout(() => this.replay(held.ids), 0);
      }
      entry = held?.entry;
    } else if (ITEMIZED[product]) {
      const itemized = this.itemized<T>(request);
      this.asked.push({ product, params, ...(asksFrom === undefined ? {} : { asksFrom }), served: itemized !== null, at: performance.now(), lane, head: this.hourOf() });
      await Promise.resolve();
      if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
      if (!itemized) throw new Error(`Not recorded: ${product} ${stableJson(params).slice(0, 400)}`);
      return itemized;
    } else if (product === 'follow-clock') {
      const anchors = this.anchors.get(clockKey(params)) ?? [];
      for (const anchor of anchors) {
        if (anchor.at > this.at) break;
        entry = anchor;
      }
      const playing = (params as FollowParams).playing;
      if (!entry || (playing && !(entry.args!.query.params as FollowParams).playing)) {
        entry = anchors.find(anchor => anchor.at >= this.at && anchor.at <= this.windowEnd
          && (anchor.args!.query.params as FollowParams).playing === playing) ?? entry;
      }
    } else if (product === 'unfollow') {
      const { id } = params as { id: number };
      for (const [recorded, local] of this.ids) if (local === id) this.ids.delete(recorded);
      entry = this.unfollow;
    } else entry = this.replies.get(productKey(product, params));
    this.asked.push({ product, params, ...(asksFrom === undefined ? {} : { asksFrom }), served: entry !== undefined, at: performance.now(), lane, head: this.hourOf() });
    await Promise.resolve();
    if (signal.aborted) throw new DOMException('Request cancelled', 'AbortError');
    if (!entry) throw new Error(`Not recorded: ${product} ${stableJson(params).slice(0, 400)}`);
    if (!entry.ok) throw queryFailure(entry.reply as QueryError);
    return { ...(entry.reply as ProductReply<T>), sourceId: request.sourceId, revision: request.revision };
  }

  private itemized<T>(request: ProductRequest): ProductReply<T> | null {
    const { asks, answers, place, item: named } = ITEMIZED[request.product]!;
    const { [asks]: listed, ...rest } = request.params as Record<string, unknown>;
    const data: Record<string, unknown> = {};
    const warnings = new Set<string>();
    for (const [at, item] of (listed as unknown[]).entries()) {
      const held = this.items.get(productKey(request.product, { ...rest, [asks]: item }));
      if (!held) return null;
      for (const [name, value] of Object.entries(held.reply.data)) {
        if (answers.includes(name)) {
          const mine = (value as Record<string, unknown>[]).flatMap(answer => place !== undefined
            ? answer[place] === held.at ? [{ ...answer, [place]: at }] : []
            : stableJson(answer[named!]) === stableJson(held.item) ? [answer] : []);
          data[name] = [...(data[name] as unknown[] | undefined ?? []), ...mine];
        } else if (name === 'warnings') for (const warning of value as string[]) warnings.add(warning);
        else data[name] = value;
      }
    }
    for (const name of answers) data[name] ??= [];
    data.warnings = [...warnings];
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: data as T };
  }

  async openFollow(ports: readonly MessagePort[]) {
    this.ports = ports;
    for (const port of ports) port.onmessage = () => {};
  }

  private readonly stamp = resetStamps();

  private send(mapped: Uint8Array) {
    const resets = this.stamp(mapped);
    const split = splitForLanes(mapped.slice(), this.ports.length);
    if (split.kind === 'failure') { for (const port of this.ports) port.postMessage({ kind: 'failure', reason: split.reason }); return; }
    split.lanes.forEach((bytes, lane) => { if (bytes) this.ports[lane]!.postMessage(packet(split.kind, bytes, resets), [bytes.buffer]); });
  }

  private replay(ids: ReadonlySet<number>): void {
    for (const bytes of this.window) {
      const mapped = remapMessage(upgradeRecorded(bytes), id => ids.has(id) ? this.ids.get(id) : undefined);
      if (mapped) this.send(mapped);
    }
  }

  deliver(bytes: Uint8Array): boolean {
    const mapped = remapMessage(upgradeRecorded(bytes), id => this.ids.get(id));
    if (!mapped) return false;
    this.send(mapped);
    return true;
  }

  async openSource(): Promise<DashboardSource> { throw new Error('A recorded client opens nothing: its source is the recording\'s'); }
  async releaseSource(): Promise<void> {}
  async setMapViews(): Promise<void> {}
  async onMapStates(): Promise<() => void> { return () => {}; }
}

export class Recorded {
  readonly file: string;
  readonly header: RecordedHeader;
  readonly entries: readonly RecordedEntry[];
  constructor(file: string, header: RecordedHeader, entries: readonly RecordedEntry[]) { this.file = file; this.header = header; this.entries = entries; }

  get source(): DashboardSource {
    const open = this.entries.find((entry): entry is Extract<RecordedEntry, { t: 'open' }> => entry.t === 'open' && entry.ok);
    if (!open) throw new Error(`${this.file} holds no opened recording`);
    const source = open.reply as SourceReply;
    return { id: source.id, path: source.path, revision: source.revision, metadata: { ...source.metadata, revision: source.revision } };
  }

  marks(label: string, page?: string): RecordedMark[] {
    return this.entries.filter((entry): entry is RecordedMark => entry.t === 'mark' && entry.label === label && (page === undefined || entry.page === page));
  }

  mark(label: string, page: string, where: (mark: RecordedMark) => boolean = () => true): RecordedMark {
    const found = this.marks(label, page).find(where);
    if (!found) throw new Error(`${this.file} holds no "${label}" of ${page}: record the replies again with the quick tier's glance and playback`);
    return found;
  }

  client(): RecordedClient { return new RecordedClient(this.entries); }

  messages(from: RecordedMark, to: RecordedMark): { at: number; hour: number | null; bytes: Uint8Array }[] {
    return this.entries.filter((entry): entry is FollowEntry => entry.t === 'follow' && entry.at >= from.at && entry.at <= to.at)
      .map(entry => { const bytes = bytesOf(entry.bytes); return { at: entry.at, hour: messageHour(bytes), bytes }; });
  }
}
