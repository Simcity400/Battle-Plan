import { applyStripPatch, decodeState, readStrip, type StripState } from './decode.ts';
import type { StripScenePlan, StripTrack, StripWork } from './scrub.ts';
import type { IdentityAsk, IdentitySnapshot } from '../../families/identity/types.ts';

export type StripUpdate =
  | { kind: 'append'; slot: string; fine: boolean; hours: readonly number[]; bytes: Uint8Array }
  | { kind: 'mark'; slot: string; fine: boolean; hours: readonly number[] }
  | { kind: 'scenes'; page: number; fine: boolean; frames: readonly [hour: number, scene: StripScenePlan][]; track?: StripTrack }
  | { kind: 'identities'; fine: boolean; snapshot?: IdentitySnapshot | number }
  | { kind: 'discover'; fine: boolean; hour: number; measure: boolean }
  | { kind: 'drop'; fine: boolean };

export interface StripReceipt { bytes: number; coarseKeys: number; keys: number; identities?: readonly IdentityAsk[]; unresolved?: boolean }
export const EMPTY_STRIP_RECEIPT: Readonly<StripReceipt> = { bytes: 0, coarseKeys: 0, keys: 0 };

type Edge = readonly [hour: number, patch: Uint8Array];
const level = () => ({ edges: new Map<number, Edge[]>(), missing: new Set<number>(), bytes: 0 });

export class StripStore {
  private whole: { hour: number; bytes: Uint8Array } | null = null;
  private coarse = level();
  private fine = level();
  private cursor: { hour: number; state: StripState } | null = null;
  patchesApplied = 0;

  get bytes() { return this.coarse.bytes + this.fine.bytes; }

  get counts() {
    const keys = new Set(this.coarse.edges.keys());
    this.coarse.missing.forEach(hour => keys.add(hour));
    if (this.whole) keys.add(this.whole.hour);
    const coarseKeys = keys.size;
    this.fine.edges.forEach((_edges, hour) => keys.add(hour));
    this.fine.missing.forEach(hour => keys.add(hour));
    return { coarseKeys, keys: keys.size };
  }

  mark(hours: readonly number[], fine: boolean) {
    const held = fine ? this.fine : this.coarse;
    hours.forEach(hour => held.missing.add(hour));
  }

  append(hours: readonly number[], bytes: Uint8Array, fine: boolean) {
    const { whole, patches } = readStrip(bytes);
    const first = hours[0]!;
    if (hours.length !== patches.length + 1 || (whole ? this.whole !== null || fine : !this.has(first))) {
      throw new Error('Dashboard strip has no matching anchor');
    }
    const held = fine ? this.fine : this.coarse;
    if (whole) this.whole = { hour: first, bytes: whole };
    const append = (from: number, to: number, patch: Uint8Array) => {
      let edges = held.edges.get(from);
      if (!edges) held.edges.set(from, edges = []);
      edges.push([to, patch]);
    };
    patches.forEach(([forward, backward], i) => {
      append(hours[i]!, hours[i + 1]!, forward);
      append(hours[i + 1]!, hours[i]!, backward);
    });
    held.bytes += bytes.byteLength;
  }

  dropFine() {
    if (this.cursor && !this.has(this.cursor.hour, false)) this.cursor = null;
    this.fine = level();
  }

  private has(hour: number, fine = true) {
    return this.whole?.hour === hour || this.coarse.edges.has(hour) || (fine && this.fine.edges.has(hour));
  }

  at(hour: number, work?: StripWork): StripState | null | undefined {
    if (this.coarse.missing.has(hour) || this.fine.missing.has(hour)) return null;
    if (!this.whole || !this.has(hour)) return undefined;
    try {
      this.cursor ??= { hour: this.whole.hour, state: decodeState(this.whole.bytes, work) };
      const from = this.cursor.hour;
      if (from === hour) return this.cursor.state;
      const via = new Map<number, Edge>();
      const queue = [from];
      for (let i = 0; i < queue.length && !via.has(hour); i++) {
        const at = queue[i]!;
        for (const [next, patch] of [...(this.coarse.edges.get(at) ?? []), ...(this.fine.edges.get(at) ?? [])]) {
          if (next === from || via.has(next)) continue;
          via.set(next, [at, patch]);
          queue.push(next);
        }
      }
      const route: Uint8Array[] = [];
      for (let at = hour; at !== from;) {
        const edge = via.get(at);
        if (!edge) throw new Error('Disconnected dashboard strip keyframe');
        const [previous, patch] = edge;
        route.push(patch);
        at = previous;
      }
      let state = this.cursor.state;
      for (const patch of route.reverse()) state = applyStripPatch(state, patch, work);
      this.patchesApplied += route.length;
      this.cursor = { hour, state };
      return state;
    } catch (error) {
      this.cursor = null;
      throw error;
    }
  }
}

export class WidgetStrips {
  readonly slots = new Map<string, StripStore>();
  page: number | undefined;
  track: StripTrack | undefined;
  private coarse = new Map<number, StripScenePlan>();
  private fine = new Map<number, StripScenePlan>();
  private coarseBytes = 0;
  private fineBytes = 0;

  scene(hour: number, fine: boolean) {
    return this.coarse.get(hour) ?? (fine ? this.fine.get(hour) : undefined);
  }

  update(update: StripUpdate): StripReceipt {
    let slot: StripStore | undefined;
    if (update.kind === 'drop') {
      this.fine.clear();
      this.fineBytes = 0;
      if (update.fine) for (const strip of this.slots.values()) strip.dropFine();
      else {
        this.page = undefined;
        this.track = undefined;
        this.coarse.clear();
        this.coarseBytes = 0;
        this.slots.clear();
      }
    } else if (update.kind === 'scenes') {
      this.page = update.page;
      this.track = update.track;
      const bytes = new TextEncoder().encode(JSON.stringify(update.fine || !update.track ? update.frames : [update.frames, update.track])).byteLength;
      if (update.fine) { this.fine = new Map(update.frames); this.fineBytes = bytes; }
      else { this.coarse = new Map(update.frames); this.coarseBytes = bytes; }
    } else if (update.kind === 'append' || update.kind === 'mark') {
      slot = this.slots.get(update.slot);
      if (!slot) this.slots.set(update.slot, slot = new StripStore());
      if (update.kind === 'append') slot.append(update.hours, update.bytes, update.fine);
      else slot.mark(update.hours, update.fine);
    }
    return { ...(slot?.counts ?? EMPTY_STRIP_RECEIPT), bytes: [...this.slots.values()].reduce((sum, strip) => sum + strip.bytes, this.coarseBytes + this.fineBytes) };
  }
}
