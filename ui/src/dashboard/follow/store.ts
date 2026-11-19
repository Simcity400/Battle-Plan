import { canonical } from './canonical.ts';
import type { PageReading } from '../../draw/followed';
import { perfLog } from '../../shell/perf.ts';
import { busy } from '../../shell/busy.ts';
import { engine } from '../../engine/frame.ts';
import type { FollowedAnswer, FollowedPanel } from './message.ts';

/** A followed query as the service names it (`at`-shaped); compared by its canonical JSON. */
export type FollowQuery = Readonly<Record<string, unknown>>;

/** The service's connection: `follow` and `unfollow` go out, messages come in. */
export interface FollowTransport {
  /** Follows `query` as `id`; resolves to whether the service follows it, and why not. */
  follow(id: number, query: FollowQuery): Promise<Availability> | void;
  unfollow(id: number): void;
  /** Starts handing each message to `receive`; the returned function stops it. */
  listen(receive: (reading: PageReading) => void, fail: (reason: string) => void): () => void;
}

/** Whether the service follows a query: until it says, pending; when it does not, the widget keeps asking as
 * before (the fallback), for `reason`. */
export type Availability = { available: true } | { available: false; reason: string };

export interface FollowEntry {
  readonly id: number;
  values?: Map<string, number | string>;
  coverage?: { start: number; through: number };
  status: 'pending' | 'followed' | 'unavailable';
  /** Why it is not followed. */
  reason?: string;
  /** The newest answer, and the hour it is at; none before the first. */
  answer: FollowedAnswer | undefined;
  /** A panel's newest reply and text: pushed only when it differs. */
  panel?: FollowedPanel;
  hour: number;
  /** The hour its answer is current through: the newest message of its playhead, which carries every answer that
   * changed, so an answer it does not resend still holds there. Its hour shown. */
  through: number;
  /** Moves with every answer, and when a message moves `through`. */
  version: number;
}

/** The hour a widget painting `entry` shows: none before its first answer. */
export const shownThrough = (entry: FollowEntry): number | null => entry.answer === undefined && entry.panel === undefined ? null : entry.through;

/** The timeline a query names: beside a panel's params, or inside a series' or product's. */
export function queryBranch(query: FollowQuery): string | null {
  const branch = query.branchId ?? (query.params as { branchId?: unknown } | undefined)?.branchId;
  return typeof branch === 'string' ? branch : null;
}

export interface FollowHandle {
  readonly entry: FollowEntry;
  /** Lets this holder's share of the follow go; the last one unfollows. */
  release(): void;
}

export interface Painter {
  /** Off screen it is not painted; shown again it is painted at the next frame if behind. */
  setVisible(visible: boolean): void;
  /** Painted again at the next frame: what it paints into changed (rows rendered). */
  repaint(): void;
  dispose(): void;
}

/** Schedules `run` once at the next animation frame, given the frame's time where it has one. */
export type FrameScheduler = (run: (now?: number) => void) => void;

/** The engine's paint phase: a landing in the data phase is painted in the same frame. */
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
  /** The widget it paints for, timed as its painting in a measured run. */
  widget: string | null;
}

export class FollowStore {
  private readonly byKey = new Map<string, Held>();
  private readonly byId = new Map<number, Held>();
  private readonly due = new Set<Painting>();
  private readonly structure = new Map<number, Set<() => void>>();
  /** Each panel's listeners for its live cells: told when its reply or one of its cells' groups lands. */
  private readonly liveListeners = new Map<number, Set<() => void>>();
  private readonly ids: () => number;
  private framed = false;
  /** In flight while a paint pass is due. */
  private painting: (() => void) | null = null;
  /** Why pushed answers stopped: every query is unavailable for it. */
  private failed: string | null = null;
  private stop?: () => void;
  private readonly transport: FollowTransport;
  private readonly frame: FrameScheduler;
  /** Messages read, paint passes run, and the painters each ran. */
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

  /** A share of the follow of `query`: followed now unless another holder already follows it. */
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

  /** `paint` writes the entry's moving parts; it runs in an animation frame, at most once a frame,
   * when the entry's version moved since it last ran and it is visible. */
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

  /** Told when `entry`'s answer changes; for structure (which rows), not values. */
  watch(entry: FollowEntry, changed: () => void): () => void {
    let listeners = this.structure.get(entry.id);
    if (!listeners) this.structure.set(entry.id, listeners = new Set());
    listeners.add(changed);
    return () => { listeners.delete(changed); if (!listeners.size) this.structure.delete(entry.id); };
  }

  /** Told when `entry`'s live cells may have moved: its panel's reply landed, or an answer of one of its cells. */
  watchLive(entry: FollowEntry, changed: () => void): () => void {
    let listeners = this.liveListeners.get(entry.id);
    if (!listeners) this.liveListeners.set(entry.id, listeners = new Set());
    listeners.add(changed);
    return () => { listeners.delete(changed); if (!listeners.size) this.liveListeners.delete(entry.id); };
  }

  seek(branch: string, retainThrough?: number) {
    for (const held of this.byId.values()) if (held.branch === branch) {
      const entry = held.entry;
      const { start, through } = entry.coverage ?? { start: entry.hour, through: entry.through };
      if (retainThrough !== undefined && start <= retainThrough && retainThrough <= through) continue;
      entry.coverage = undefined;
      entry.panel = undefined; entry.answer = undefined; entry.values = undefined;
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
