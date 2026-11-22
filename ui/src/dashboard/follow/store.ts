import { canonical } from './canonical.ts';
import type { PageReading } from '../../draw/followed';
import { perfLog } from '../../shell/perf.ts';
import { busy } from '../../shell/busy.ts';
import { engine } from '../../engine/frame.ts';
import type { FollowedAnswer, FollowedPanel } from './message.ts';

import type { FollowQuery } from '../regionTime';
export type { FollowQuery };

export interface FollowTransport {
  follow(id: number, query: FollowQuery): Promise<Availability> | void;
  unfollow(id: number): void;
  listen(receive: (reading: PageReading) => void, fail: (reason: string) => void): () => void;
}

export type Availability = { available: true } | { available: false; reason: string };

export interface Landed { panel: FollowedPanel; values?: Map<string, number | string>; hour: number; through: number }

export interface FollowEntry {
  readonly id: number;
  values?: Map<string, number | string>;
  coverage?: { start: number; through: number };
  status: 'pending' | 'followed' | 'unavailable';
  reason?: string;
  answer: FollowedAnswer | undefined;
  panel?: FollowedPanel;
  landed?: Landed | null;
  hour: number;
  through: number;
  version: number;
}

export const shownThrough = (entry: FollowEntry): number | null => entry.answer === undefined && entry.panel === undefined ? null : entry.through;

export const showsAt = (reading: Pick<PageReading, 'panel' | 'answer' | 'coverage'>, paused: number | null) => paused === null || !reading.panel && !reading.answer
  || !!reading.coverage && reading.coverage.start <= paused && paused <= reading.coverage.through;

export function queryBranch(query: FollowQuery): string | null {
  const branch = query.branchId ?? (query.params as { branchId?: unknown } | undefined)?.branchId;
  return typeof branch === 'string' ? branch : null;
}

export interface FollowHandle {
  readonly entry: FollowEntry;
  release(): void;
}

export interface Painter {
  setVisible(visible: boolean): void;
  repaint(): void;
  dispose(): void;
}

export type FrameScheduler = (run: (now?: number) => void) => void;

const defaultFrame: FrameScheduler = run => { engine.next('paint', frame => run(frame.now)); };

interface Held {
  entry: FollowEntry;
  key: string;
  branch: string | null;
  holders: number;
  painters: Set<Painting>;
}

interface Painting {
  held: Held;
  paint: (entry: FollowEntry) => unknown;
  visible: boolean;
  painted: number;
  widget: string | null;
}

export class FollowStore {
  private readonly byKey = new Map<string, Held>();
  private readonly byId = new Map<number, Held>();
  private readonly due = new Set<Painting>();
  private readonly structure = new Map<number, Set<() => void>>();
  private readonly liveListeners = new Map<number, Set<() => void>>();
  private readonly ids: () => number;
  private framed = false;
  private painting: (() => void) | null = null;
  private failed: string | null = null;
  private stop?: () => void;
  private readonly transport: FollowTransport;
  private readonly frame: FrameScheduler;
  readonly counts = { frames: 0, paints: 0 };

  constructor(transport: FollowTransport, frame: FrameScheduler = defaultFrame, ids?: () => number) {
    let next = 1;
    this.ids = ids ?? (() => next++);
    this.transport = transport;
    this.frame = frame;
  }

  private listen() {
    this.stop ??= this.transport.listen(reading => this.receive(reading), reason => {
      this.failed = reason;
      for (const held of this.byId.values()) this.settle(held, { available: false, reason });
    });
  }

  acquire(query: FollowQuery): FollowHandle {
    const key = canonical(query);
    let held = this.byKey.get(key);
    if (!held) {
      this.listen();
      held = { entry: { id: this.ids(), status: 'pending', answer: undefined, hour: 0, through: 0, version: 0 }, key, branch: queryBranch(query),
        holders: 0, painters: new Set() };
      this.byKey.set(key, held);
      this.byId.set(held.entry.id, held);
      const asked = held;
      perfLog?.follow(held.entry.id, 'pending', typeof query.product === 'string' ? query.product : typeof query.panel === 'string' ? query.panel
        : typeof query.head === 'string' ? query.head : null);
      const told = this.failed === null ? this.transport.follow(held.entry.id, query) : Promise.resolve<Availability>({ available: false, reason: this.failed });
      if (told) told.then(availability => this.settle(asked, availability),
        error => this.settle(asked, { available: false, reason: error instanceof Error ? error.message : String(error) }));
      else held.entry.status = 'followed';
    }
    held.holders++;
    let released = false;
    const owner = held;
    return { entry: held.entry, release: () => {
      if (released) return;
      released = true;
      if (--owner.holders > 0) return;
      this.byKey.delete(owner.key);
      this.byId.delete(owner.entry.id);
      for (const painting of owner.painters) this.due.delete(painting);
      this.liveListeners.delete(owner.entry.id);
      this.structure.delete(owner.entry.id);
      perfLog?.follow(owner.entry.id, 'released');
      this.transport.unfollow(owner.entry.id);
      if (!this.byId.size) this.close();
    } };
  }

  painter(entry: FollowEntry, paint: (entry: FollowEntry) => unknown, visible = true, widget: string | null = null): Painter {
    const held = this.byId.get(entry.id);
    if (!held) throw new Error('A painter of a query no longer followed.');
    const painting: Painting = { held, paint, visible, painted: -1, widget };
    held.painters.add(painting);
    this.consider(painting);
    return {
      setVisible: shown => { painting.visible = shown; if (shown) this.consider(painting); else this.due.delete(painting); },
      repaint: () => { painting.painted = -1; this.consider(painting); },
      dispose: () => { held.painters.delete(painting); this.due.delete(painting); },
    };
  }

  watch(entry: FollowEntry, changed: () => void): () => void {
    let listeners = this.structure.get(entry.id);
    if (!listeners) this.structure.set(entry.id, listeners = new Set());
    listeners.add(changed);
    return () => { listeners.delete(changed); if (!listeners.size) this.structure.delete(entry.id); };
  }

  watchLive(entry: FollowEntry, changed: () => void): () => void {
    let listeners = this.liveListeners.get(entry.id);
    if (!listeners) this.liveListeners.set(entry.id, listeners = new Set());
    listeners.add(changed);
    return () => { listeners.delete(changed); if (!listeners.size) this.liveListeners.delete(entry.id); };
  }

  seek(branch: string, retainThrough?: number, stepped = false) {
    if (retainThrough !== undefined && !stepped) return;
    for (const held of this.byId.values()) if (held.branch === branch) {
      const entry = held.entry;
      if (retainThrough !== undefined && showsAt(entry, retainThrough)) continue;
      entry.coverage = undefined;
      entry.panel = undefined; entry.answer = undefined; entry.values = undefined;
      if (retainThrough === undefined && entry.landed) entry.landed = null;
      this.changed(held);
    }
  }

  private receive(reading: PageReading) {
    const held = this.byId.get(reading.id);
    if (!held || held.entry.status === 'unavailable') return;
    const entry = held.entry;
    entry.hour = reading.hour; entry.through = reading.through; entry.coverage = reading.coverage;
    if (reading.panel !== undefined) entry.panel = reading.panel ?? undefined;
    if (reading.answer !== undefined) entry.answer = reading.answer ?? undefined;
    if (reading.values !== undefined) entry.values = reading.values ?? undefined;
    if (entry.panel) entry.landed = { panel: entry.panel, values: entry.values, hour: entry.hour, through: entry.through };
    this.changed(held);
  }

  private changed(held: Held) {
    held.entry.version++;
    this.tellLive(held);
    for (const changed of this.structure.get(held.entry.id) ?? []) changed();
    for (const painting of held.painters) this.consider(painting);
  }

  get(id: number): FollowEntry | undefined {
    return this.byId.get(id)?.entry;
  }

  close(): void {
    this.stop?.();
    this.stop = undefined;
    this.failed = null;
  }

  private settle(held: Held, availability: Availability): void {
    if (this.byId.get(held.entry.id) !== held) return;
    if (this.failed !== null) availability = { available: false, reason: this.failed };
    if (!availability.available) {
      held.entry.answer = undefined;
      held.entry.panel = undefined;
      held.entry.values = undefined;
      held.entry.landed = undefined;
      this.tellLive(held);
    }
    held.entry.status = availability.available ? 'followed' : 'unavailable';
    held.entry.reason = availability.available ? undefined : availability.reason;
    perfLog?.follow(held.entry.id, held.entry.status, undefined, held.entry.reason);
    const listeners = this.structure.get(held.entry.id);
    if (listeners) for (const changed of [...listeners]) changed();
  }

  private tellLive(held: Held): void {
    const listeners = this.liveListeners.get(held.entry.id);
    if (listeners) for (const changed of [...listeners]) changed();
  }

  private consider(painting: Painting): void {
    if (!painting.held.entry.answer && !painting.held.entry.panel) return;
    if (!painting.visible || painting.painted === painting.held.entry.version) return;
    this.due.add(painting);
    if (!this.framed) {
      this.framed = true;
      this.painting = busy('follow');
      this.frame(() => this.paintFrame());
    }
  }

  private paintFrame(): void {
    this.framed = false;
    this.counts.frames++;
    const due = [...this.due];
    this.due.clear();
    const end = this.painting;
    this.painting = null;
    try { this.paintDue(due); } finally { end?.(); }
  }

  private paintDue(due: readonly Painting[]): void {
    for (const painting of due) {
      const { entry } = painting.held;
      if (!entry.answer && !entry.panel) continue;
      if (!painting.visible || painting.painted === entry.version) continue;
      const version = entry.version;
      if (painting.paint(entry) === false) this.consider(painting);
      else { painting.painted = version; this.counts.paints++; }
    }
  }
}
