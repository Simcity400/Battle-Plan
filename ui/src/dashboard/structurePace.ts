import { useCallback, useEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { slotCommits } from '../shell/commitCount.ts';
import { perfLog } from '../shell/perf.ts';
import { busy } from '../shell/busy.ts';
import { engine } from '../engine/frame.ts';
import type { EngineTask } from '../engine/frame.ts';

/** The page's one pace for structure while playing (rows appearing or leaving, their order, a widget's regions): a
 * quarter-second grid every widget shares. A widget's structure commits at most once a slot, so four times a second
 * however fast its values churn, and the changes held back are taken together at the slot's start, in one task, so
 * React commits every widget they wake at once. Paused, a structure commits at once. */
export const STRUCTURE_SLOT_MS = 250;

export const structureSlot = (now: number) => Math.floor(now / STRUCTURE_SLOT_MS);

const due = new Set<() => void>();
/** Waits in the engine's commit phase for the first frame at or past `dueAt`. */
let slotTask: EngineTask | null = null;
/** When the held runs may run, on `performance.now()`. */
let dueAt = 0;
/** In flight while runs are held for a slot. */
let holding: (() => void) | null = null;

/** The slot, in the commit phase of the first frame at or past its start: its runs are taken in one task posted from
 * there, which runs once that frame is drawn, so the page's structure commits once a slot, between frames, never inside
 * one. The task commits what it takes before it ends (`flushSync`): its time and each widget's part of it go to the perf
 * log, with the frame drawn after it, whose interval shows whether the slot held that frame back. */
function slotFrame(frame: { now: number }): boolean {
  if (frame.now < dueAt) return true;
  slotTask?.stop();
  slotTask = null;
  if (!posted) { posted = true; afterFrame(takeSlot); }
  return false;
}

/** Whether the slot's task is posted and has not run. */
let posted = false;

function takeSlot(): void {
  posted = false;
  const runs = [...due];
  due.clear();
  const end = holding;
  holding = null;
  const started = performance.now();
  try {
    const taken = slotCommits(() => flushSync(() => inStructureSlot(() => { for (const run of runs) run(); })));
    perfLog?.slot(performance.now() - started, taken.commits, taken.widgets, taken.causes);
  } finally { end?.(); }
}

/** Runs `run` in a task of its own, after the current one: posted from a frame, once that frame is drawn. */
const afterFrame: (run: () => void) => void = typeof MessageChannel === 'function' ? (() => {
  const channel = new MessageChannel();
  const queued: (() => void)[] = [];
  channel.port1.onmessage = () => { for (const run of queued.splice(0)) run(); };
  // Outside a page (node), an open port would keep the process alive.
  (channel.port1 as { unref?: () => void }).unref?.();
  return (run: () => void) => { queued.push(run); channel.port2.postMessage(0); };
})() : run => { setTimeout(run, 0); };

let slotOpen = false;

/** Runs `run` as the slot's task: every structure offered inside it is taken. */
export function inStructureSlot(run: () => void): void {
  const was = slotOpen;
  slotOpen = true;
  try { run(); } finally { slotOpen = was; }
}

/** Whether a structure of a clock that `playing` may be taken now: paused at once; playing only inside the slot's task,
 * so the whole page's structure commits together, once a slot, after the frame has painted. */
export const structureOpen = (playing: boolean = pagePlaying()): boolean => !playing || slotOpen;


/** Runs `run` at the start of the next slot after `now`, with every other run held for it, in one task, never before
 * it; the returned function cancels it. A run already held is held once. */
export function atNextSlot(run: () => void, now = performance.now()): () => void {
  due.add(run);
  holding ??= busy('updates');
  if (slotTask === null) {
    dueAt = performance.now() + Math.max(0, Math.ceil((structureSlot(now) + 1) * STRUCTURE_SLOT_MS - now));
    slotTask = engine.every('commit', slotFrame);
  }
  return () => {
    due.delete(run);
    if (!due.size && slotTask !== null) { slotTask.stop(); slotTask = null; }
    if (!due.size) { holding?.(); holding = null; }
  };
}

/** The page's playback clock: the one owner of when anything but the reader's own input commits while it plays. Every
 * followed structure without a clock of its own (`useFollow`) and every deferred data update (`inFrame`) is held for the
 * next slot by it, and taken in the one task `atNextSlot` runs, with every other paced structure of the page, so React
 * commits once a slot for the whole page while it plays; paused, at once. */
let pageClock: PaceClock | null = null;
let stopPage: (() => void) | null = null;
const onPause = new Set<() => void>();

/** Makes `clock` the page's (null: none); told at each pause, seek or step, what waits for a slot is taken at once. */
export function setPageClock(clock: PaceClock | null): void {
  stopPage?.();
  stopPage = null;
  pageClock = clock;
  if (clock) stopPage = clock.subscribe(() => { if (!clock.getSnapshot().playing) for (const run of [...onPause]) run(); });
}

/** The page's clock, for a structure that has none of its own. */
export const pagePace = (): PaceClock | null => pageClock;

/** Whether the page's clock plays. */
export const pagePlaying = (): boolean => pageClock?.getSnapshot().playing === true;

/** Runs `run` whenever the page's clock stops, seeks or steps; the returned function stops it. */
export function atPagePause(run: () => void): () => void {
  onPause.add(run);
  return () => { onPause.delete(run); };
}

/** The parts of the playback clock the pace reads: whether it plays, and its anchors (play, pause, seek, step). */
export interface PaceClock {
  getSnapshot(): { playing: boolean };
  subscribe(listener: () => void): () => void;
}

/** What one owner of a paced structure holds: the value committed, and the one held back for the next slot. */
export class StructurePace<T> {
  held: T;
  private next: { value: T } | null = null;
  private cancel: (() => void) | null = null;
  private readonly same: (held: T, next: T) => boolean;
  private readonly apply: (value: T) => void;
  constructor(initial: T, same: (held: T, next: T) => boolean, apply: (value: T) => void) {
    this.held = initial;
    this.same = same;
    this.apply = apply;
  }

  /** Offers `value`: taken now when it differs and a structure may be taken (`structureOpen`); else held for the next
   * slot, where the newest offered is taken. The value held again cancels what waits. */
  offer(value: T, playing: boolean): void {
    if (this.same(this.held, value)) { this.drop(); return; }
    if (this.next && this.same(this.next.value, value)) return;
    if (structureOpen(playing)) { this.take(value); return; }
    this.next = { value };
    this.cancel ??= atNextSlot(() => { this.cancel = null; const next = this.next; if (next) this.take(next.value); });
  }

  /** Takes what waits now (the clock stopped, stepped or sought). */
  settle(): void {
    const next = this.next;
    if (next) this.take(next.value);
  }

  drop(): void {
    this.next = null;
    this.cancel?.();
    this.cancel = null;
  }

  private take(value: T): void {
    this.drop();
    this.held = value;
    this.apply(value);
  }
}

/** State a widget sets from its painter or a callback (its rows, their order, its regions): while `clock` plays, it
 * commits at most once a slot of the shared grid, the newest set taken at the slot's start; paused, or at a pause, a
 * seek or a step, at once. A value `same` as the one held commits nothing. */
export function useStructureState<T>(clock: PaceClock, initial: T, same: (held: T, next: T) => boolean = Object.is): [T, (next: T) => void] {
  const [value, setValue] = useState(initial);
  const pace = useRef<StructurePace<T> | null>(null);
  pace.current ??= new StructurePace(initial, same, next => setValue(() => next));
  const set = useCallback((next: T) => { pace.current!.offer(next, clock.getSnapshot().playing); }, [clock]);
  useEffect(() => {
    const held = pace.current!;
    const stop = clock.subscribe(() => { if (!clock.getSnapshot().playing) held.settle(); });
    return () => { stop(); held.drop(); };
  }, [clock]);
  return [value, set];
}
