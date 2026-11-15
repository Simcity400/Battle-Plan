/** The dashboard's one frame loop. Every display frame runs six phases in order, and nothing else schedules frame work:
 * - `clock`: the playback clock advances (computes its position, tells no one);
 * - `data`: everything that arrived since the last frame lands, in arrival order, in one batch;
 * - `measure`: the only place a layout read belongs, before anything in the frame writes;
 * - `paint`: every moving part draws from the data the frame holds (writes only, never a layout read);
 * - `commit`: React (the structure slot while playing, frame updates while paused);
 * - `after`: one-shots that wait for the next frame.
 * Work added during a phase for a later phase of the same frame runs in that frame; for the same or an earlier phase, in
 * the next (a task later in its phase may still run in this one). Without animation frames (tests, a hidden window) it ticks on a timer. */

export const PHASES = ['clock', 'data', 'measure', 'paint', 'commit', 'after'] as const;
export type Phase = typeof PHASES[number];

export interface EngineFrame {
  /** The frame's time, on `performance.now()`'s clock. */
  readonly now: number;
  /** Since the previous frame, at most `MAX_DT`. */
  readonly dt: number;
  /** Counts frames from 1. */
  readonly index: number;
  readonly phase: Phase;
}

export interface FrameHost {
  /** Calls `run` once, at the next frame. */
  request(run: (now: number) => void): void;
}

/** A continuing task: runs every frame in its phase while it returns true, else sleeps until woken. */
export interface EngineTask {
  /** Runs it in the next frame's phase (or this frame's, if its phase is still to come). */
  wake(): void;
  stop(): void;
}

/** Longest step reported, so a tab returning from the background does not jump. */
const MAX_DT = 64;
const FALLBACK_MS = 16;

interface Task { phase: number; run: (frame: EngineFrame) => boolean | void; awake: boolean; stopped: boolean }

export class FrameEngine {
  private readonly host: FrameHost;
  /** Per phase, the one-shots and the awake tasks due in the next frame that reaches it. */
  private readonly once: ((frame: EngineFrame) => void)[][] = PHASES.map(() => []);
  private readonly tasks: Set<Task>[] = PHASES.map(() => new Set());
  private asked = false;
  private last: number | null = null;
  private index = 0;
  /** The phase running now (its index), -1 outside a frame. */
  private running = -1;
  private current: EngineFrame | null = null;

  constructor(host: FrameHost) { this.host = host; }

  /** The frame running now, with its phase; null between frames. */
  get frame(): EngineFrame | null { return this.current; }

  /** Runs `run` once, in `phase` of the next frame that reaches it; the returned function cancels it. */
  next(phase: Phase, run: (frame: EngineFrame) => void): () => void {
    const list = this.once[PHASES.indexOf(phase)];
    list.push(run);
    this.schedule();
    return () => { const at = list.indexOf(run); if (at >= 0) list.splice(at, 1); };
  }

  /** A task run in `phase` of every frame while it returns true; it starts awake. */
  every(phase: Phase, run: (frame: EngineFrame) => boolean | void): EngineTask {
    const task: Task = { phase: PHASES.indexOf(phase), run, awake: true, stopped: false };
    this.tasks[task.phase].add(task);
    this.schedule();
    return {
      wake: () => { if (task.stopped) return; task.awake = true; this.schedule(); },
      stop: () => { task.stopped = true; this.tasks[task.phase].delete(task); },
    };
  }

  /** A queue landed in the data phase: items pushed between frames reach `land` together, in order, once a frame. */
  batch<T>(land: (items: readonly T[]) => void): (item: T) => void {
    let items: T[] = [];
    let due = false;
    const flush = () => { due = false; const landing = items; items = []; land(landing); };
    return item => {
      items.push(item);
      if (!due) { due = true; this.next('data', flush); }
    };
  }

  private hasWork(): boolean {
    for (let phase = 0; phase < PHASES.length; phase++) {
      if (this.once[phase].length) return true;
      for (const task of this.tasks[phase]) if (task.awake) return true;
    }
    return false;
  }

  private schedule(): void {
    if (this.asked || this.running >= 0) return;
    this.asked = true;
    this.host.request(now => this.tick(now));
  }

  /** One frame: each phase's one-shots, then its awake tasks; a phase still to come takes what earlier phases add. */
  tick(now: number): void {
    this.asked = false;
    const dt = this.last === null ? FALLBACK_MS : Math.min(MAX_DT, Math.max(0, now - this.last));
    this.last = now;
    const index = ++this.index;
    try {
      for (let phase = 0; phase < PHASES.length; phase++) {
        this.running = phase;
        const frame: EngineFrame = { now, dt, index, phase: PHASES[phase] };
        this.current = frame;
        const runs = this.once[phase].splice(0);
        for (const run of runs) {
          try { run(frame); } catch (error) { console.error(error); }
        }
        for (const task of [...this.tasks[phase]]) {
          if (!task.awake || task.stopped) continue;
          task.awake = false;
          let again: boolean | void = false;
          try { again = task.run(frame); } catch (error) { console.error(error); }
          if (again === true && !task.stopped) task.awake = true;
        }
      }
    } finally {
      this.running = -1;
      this.current = null;
    }
    if (this.hasWork()) this.schedule();
    else this.last = null;
  }

  /** Whether a frame is asked for or running. */
  get busy(): boolean { return this.asked || this.running >= 0; }
}

/** Animation frames while the page is shown; a timer without them or while the window is hidden (no frames come then). */
const pageHost: FrameHost = {
  request: run => {
    if (typeof requestAnimationFrame === 'function' && !(typeof document !== 'undefined' && document.visibilityState === 'hidden')) requestAnimationFrame(run);
    else setTimeout(() => run(performance.now()), FALLBACK_MS);
  },
};

/** The dashboard's one engine. */
export const engine = new FrameEngine(pageHost);
