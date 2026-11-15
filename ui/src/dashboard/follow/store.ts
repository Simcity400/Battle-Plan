/** Followed answers outside React: one entry per followed query, filled from the service's
 * messages (each parsed once, however many entries it updates), and the painters that write an
 * entry's moving parts into the page at most once a frame, only when its version moved and only
 * while on screen. Two widgets following the same query share one follow. A panel's live cells are followed by its
 * host under ids of their own (`liveCells.ts`): their answers land in a `CellGroup` the panel names. */
import { OUTSIDE, perfLog, timePaint } from '../../shell/perf.ts';
import { busy } from '../../shell/busy.ts';
import { engine } from '../../engine/frame.ts';
import { followFailure, isPanelMessage, parseFollowed, parsePanels } from './message.ts';
import { CellGroup, cellIds, isCellsId } from './liveCells.ts';
import type { FollowedAnswer, FollowedPanel } from './message.ts';

/** A followed query as the service names it (`at`-shaped); compared by its canonical JSON. */
export type FollowQuery = Readonly<Record<string, unknown>>;

/** The service's connection: `follow` and `unfollow` go out, messages come in. */
export interface FollowTransport {
  /** Follows `query` as `id`; resolves to whether the service follows it, and why not. */
  follow(id: number, query: FollowQuery): Promise<Availability> | void;
  unfollow(id: number): void;
  /** Starts handing each message to `receive`; the returned function stops it. */
  listen(receive: (bytes: ArrayBuffer | Uint8Array) => void): () => void;
}

/** Whether the service follows a query: until it says, pending; when it does not, the widget keeps asking as
 * before (the fallback), for `reason`. */
export type Availability = { available: true } | { available: false; reason: string };

export interface FollowEntry {
  readonly id: number;
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

/** `value` as JSON with its keys in order: one text for one query however it was written. */
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record).sort().filter(key => record[key] !== undefined).map(key => `${JSON.stringify(key)}:${canonical(record[key])}`).join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}

interface Held {
  entry: FollowEntry;
  key: string;
  branch: string | null;
  panel: boolean;
  holders: number;
  painters: Set<Painting>;
  /** A panel's: the follow ids of the groups of live cells its newest reply names. */
  cells: number[];
  /** The follow its answers come from (`crates/dashboard-follow`, `Follow::route`): none before the first. */
  route: number | null;
}

interface Painting {
  held: Held;
  paint: (entry: FollowEntry) => void;
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
  private readonly cellGroups = new Map<number, CellGroup>();
  private readonly cellOwners = new Map<number, Held>();
  /** The greatest cells id a landed panel named: an answer of a lesser id no panel names is of a group let go. */
  private named = 0;
  private readonly ids: () => number;
  private framed = false;
  /** In flight while a paint pass is due. */
  private painting: (() => void) | null = null;
  /** The hour a seek asked of a timeline, in flight until a message of that hour lands. */
  private sought: { hour: number; branch: string; end: () => void } | null = null;
  /** Why pushed answers stopped: every query is unavailable for it. */
  private failed: string | null = null;
  private stop: (() => void) | null;
  private readonly transport: FollowTransport;
  private readonly frame: FrameScheduler;
  /** Messages read, paint passes run, and the painters each ran. */
  readonly counts = { messages: 0, frames: 0, paints: 0 };

  constructor(transport: FollowTransport, frame: FrameScheduler = defaultFrame, ids?: () => number) {
    let next = 1;
    this.ids = ids ?? (() => next++);
    this.transport = transport;
    this.frame = frame;
    this.stop = transport.listen(bytes => this.receive(bytes));
  }

  /** A share of the follow of `query`: followed now unless another holder already follows it. */
  acquire(query: FollowQuery): FollowHandle {
    const key = canonical(query);
    let held = this.byKey.get(key);
    if (!held) {
      held = { entry: { id: this.ids(), status: 'pending', answer: undefined, hour: 0, through: 0, version: 0 }, key, branch: queryBranch(query),
        panel: typeof query.panel === 'string', holders: 0, painters: new Set(), cells: [], route: null };
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
      this.nameCells(owner, []);
      this.liveListeners.delete(owner.entry.id);
      perfLog?.follow(owner.entry.id, 'released');
      this.transport.unfollow(owner.entry.id);
      this.soughtStill();
    } };
  }

  /** `paint` writes the entry's moving parts; it runs in an animation frame, at most once a frame,
   * when the entry's version moved since it last ran and it is visible. */
  painter(entry: FollowEntry, paint: (entry: FollowEntry) => void, visible = true, widget: string | null = null): Painter {
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

  /** The cells followed under `id`, as their answers left them. */
  cellGroup(id: number): CellGroup | undefined {
    return this.cellGroups.get(id);
  }

  /** The page sought `hour` of `branch` while paused: in flight until a message of that hour lands, unless every query
   * followed on that timeline already holds it. The returned function ends it (the service follows nothing there). */
  seek(branch: string, hour: number): () => void {
    this.sought?.end();
    this.sought = null;
    if (!this.behind(branch, hour)) return () => {};
    const sought = { hour, branch, end: busy('follow') };
    this.sought = sought;
    return () => { sought.end(); if (this.sought === sought) this.sought = null; };
  }

  /** Whether a query followed on `branch` holds no answer at `hour`. */
  private behind(branch: string, hour: number): boolean {
    for (const held of this.byId.values()) if (held.branch === branch && held.entry.status === 'followed' && held.entry.through !== hour) return true;
    return false;
  }

  /** A message at `hour` landed: a seek waiting for it has its answers. */
  private reached(hour: number): void {
    if (this.sought?.hour !== hour) return;
    this.sought.end();
    this.sought = null;
  }

  /** A seek waits no longer once no query followed on its timeline lacks the hour (let go, or not followed after all). */
  private soughtStill(): void {
    if (!this.sought || this.behind(this.sought.branch, this.sought.hour)) return;
    this.sought.end();
    this.sought = null;
  }

  /** One message: every answer it holds lands in its entry, and their painters are due. */
  receive(bytes: ArrayBuffer | Uint8Array): void {
    this.counts.messages++;
    const failed = followFailure(bytes);
    if (failed !== null) {
      this.failed = failed;
      for (const held of this.byId.values()) this.settle(held, { available: false, reason: failed });
      this.sought?.end();
      this.sought = null;
      return;
    }
    if (isPanelMessage(bytes)) {
      const panels = parsePanels(bytes);
      if (perfLog && panels.length) perfLog.followMessage(Math.max(...panels.map(panel => panel.hour)), panels.map(panel => [panel.id, panel.hour] as const));
      for (const { id, hour, panel } of panels) {
        const held = this.byId.get(id);
        if (held) this.landPanel(held, panel, hour);
      }
      // Groups that answered before a reply named them, and that no reply will name now: ids are given in order, and the
      // replies of one message land together.
      for (const id of this.cellGroups.keys()) if (id <= this.named && !this.cellOwners.has(id)) this.cellGroups.delete(id);
      for (const { hour } of panels) this.reached(hour);
      return;
    }
    const message = parseFollowed(bytes);
    perfLog?.followMessage(message.hour, message.answers.map(answer => [answer.id, answer.hour] as const));
    // Every store of the page hears every message: one is this store's when its follow (route) is one an answer it
    // holds came from. A playhead's hours, seeks and logs are its own.
    const { route } = message;
    for (const { id } of message.answers) {
      const held = isCellsId(id) ? this.cellOwners.get(id) : this.byId.get(id);
      if (held) held.route = route;
    }
    const ours: Held[] = [];
    for (const held of this.byId.values()) if (held.route === route) ours.push(held);
    const landed = new Set<Held>();
    if (message.reset) {
      for (const held of ours) {
        if (held.entry.answer?.kind === 'log') { this.land(held, undefined, message.hour, message.hour); landed.add(held); }
      }
    }
    for (const { id, hour, answer } of message.answers) {
      if (isCellsId(id)) { this.landCells(id, answer, message.hour, route); continue; }
      const held = this.byId.get(id);
      if (!held) continue;
      this.land(held, answer, hour, message.hour);
      landed.add(held);
    }
    if (!ours.length) return;
    this.reached(message.hour);
    // A panel's cells are on its follow's playhead (a page of panels alone still hears each step); its own reply is not.
    for (const held of ours) {
      if (held.panel && held.cells.length) { this.holdCells(held, message.hour); continue; }
      if (held.panel || landed.has(held) || held.entry.through === message.hour) continue;
      held.entry.through = message.hour;
      held.entry.version++;
      for (const painting of held.painters) this.consider(painting);
    }
  }

  get(id: number): FollowEntry | undefined {
    return this.byId.get(id)?.entry;
  }

  close(): void {
    this.stop?.();
    this.stop = null;
    this.sought?.end();
    this.sought = null;
  }

  private settle(held: Held, availability: Availability): void {
    if (this.byId.get(held.entry.id) !== held) return;
    held.entry.status = availability.available ? 'followed' : 'unavailable';
    held.entry.reason = availability.available ? undefined : availability.reason;
    perfLog?.follow(held.entry.id, held.entry.status, undefined, held.entry.reason);
    if (!availability.available) this.soughtStill();
    const listeners = this.structure.get(held.entry.id);
    if (listeners) for (const changed of [...listeners]) changed();
  }

  private land(held: Held, answer: FollowedAnswer | undefined, hour: number, through: number): void {
    held.entry.answer = answer;
    held.entry.hour = hour;
    held.entry.through = through;
    held.entry.version++;
    for (const painting of held.painters) this.consider(painting);
    const listeners = this.structure.get(held.entry.id);
    if (listeners) for (const changed of [...listeners]) changed();
  }

  /** A panel lands only when its reply or refusal differs from the one held: its painters are due and its structure
   * is asked again. */
  private landPanel(held: Held, panel: FollowedPanel, hour: number): void {
    const before = held.entry.panel;
    const moved = held.entry.through !== hour;
    held.entry.hour = hour;
    held.entry.through = hour;
    if (before && before.refused === panel.refused && before.dataText === panel.dataText) {
      // The same reply of another hour (pushed again after a seek): its readers report the hour it now holds.
      if (moved) this.tellLive(held);
      return;
    }
    held.entry.panel = panel;
    held.entry.version++;
    this.nameCells(held, panel.refused === null ? cellIds(panel.data) : []);
    this.tellLive(held);
    for (const painting of held.painters) this.consider(painting);
    const listeners = this.structure.get(held.entry.id);
    if (listeners) for (const changed of [...listeners]) changed();
  }

  /** An answer of a group of a panel's cells: kept for the panel naming it, or for the panel that will (its reply may
   * come after its cells' first answer); one of a group let go is dropped. */
  private landCells(id: number, answer: FollowedAnswer, through: number, route: number): void {
    let group = this.cellGroups.get(id);
    if (!group) {
      if (id <= this.named) return;
      this.cellGroups.set(id, group = new CellGroup());
    }
    group.land(answer);
    group.through = through;
    group.route = route;
    const owner = this.cellOwners.get(id);
    if (owner) this.tellLive(owner);
  }

  /** `held`'s cells are those `ids` names now: groups it named before and names no more are let go. A group that
   * answered before it was named gives `held` its route. */
  private nameCells(held: Held, ids: readonly number[]): void {
    for (const id of held.cells) {
      if (ids.includes(id)) continue;
      this.cellGroups.delete(id);
      this.cellOwners.delete(id);
    }
    for (const id of ids) {
      const group = this.cellGroups.get(id);
      if (!group) this.cellGroups.set(id, new CellGroup());
      else if (held.route === null && group.route !== null) held.route = group.route;
      this.cellOwners.set(id, held);
      this.named = Math.max(this.named, id);
    }
    if (held.panel && (ids.length !== held.cells.length || ids.some((id, at) => id !== held.cells[at]))) perfLog?.followCells(held.entry.id, ids);
    held.cells = [...ids];
  }

  /** A message of `held`'s timeline: its cells' answers, sent or not (an unchanged one is not resent), hold through
   * its hour; its painters are woken to report it. */
  private holdCells(held: Held, hour: number): void {
    let moved = false;
    for (const id of held.cells) {
      const group = this.cellGroups.get(id);
      if (group && group.through !== null && group.through !== hour) { group.through = hour; moved = true; }
    }
    if (moved) this.tellLive(held);
  }

  /** The hour `entry`'s shown values are current through: the oldest of its cells' groups, or its reply's when it
   * names none; null before every one has answered. */
  liveThrough(entry: FollowEntry): number | null {
    const held = this.byId.get(entry.id);
    if (!held?.cells.length) return shownThrough(entry);
    let through: number | null = Infinity;
    for (const id of held.cells) {
      const group = this.cellGroups.get(id)?.through ?? null;
      through = group === null || through === null ? null : Math.min(through, group);
    }
    return through;
  }

  private tellLive(held: Held): void {
    const listeners = this.liveListeners.get(held.entry.id);
    if (listeners) for (const changed of [...listeners]) changed();
  }

  private consider(painting: Painting): void {
    if (!painting.visible || painting.painted === painting.held.entry.version) return;
    this.due.add(painting);
    if (!this.framed) {
      this.framed = true;
      this.painting = busy('follow');
      this.frame(now => this.paintFrame(now ?? OUTSIDE));
    }
  }

  private paintFrame(now: number): void {
    this.framed = false;
    this.counts.frames++;
    const due = [...this.due];
    this.due.clear();
    const end = this.painting;
    this.painting = null;
    try { this.paintDue(due, now); } finally { end?.(); }
  }

  private paintDue(due: readonly Painting[], now: number): void {
    for (const painting of due) {
      const { entry } = painting.held;
      if (!painting.visible || painting.painted === entry.version) continue;
      painting.painted = entry.version;
      this.counts.paints++;
      if (perfLog) timePaint(painting.widget, now, () => painting.paint(entry));
      else painting.paint(entry);
    }
  }
}
