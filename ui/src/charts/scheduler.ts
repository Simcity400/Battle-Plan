/** Shared canvas loop; the budget rotates waiting tasks but cannot preempt a long draw. */
import { perfLog, timePaint } from '../shell/perf.ts';
import { busy } from '../shell/busy.ts';
import { engine } from '../engine/frame.ts';

export interface FrameInfo {
  now: number;
  dt: number;
  /** The clock time by which this frame's painting should end ([`FrameHost.clock`]); none, no limit. */
  deadline?: number;
}
export interface FrameTask {
  /** Draws one frame; returns true to be called again next frame. */
  frame(info: FrameInfo): boolean;
}
export interface FrameHost {
  request(callback: (now: number) => void): number;
  /** The time a task's cost is measured with; `performance.now` without it. */
  clock?: () => number;
}

/** Longest step reported to tasks, so a tab returning from the background doesn't jump. */
const MAX_DT = 64;
/** Style, layout and compositing need the rest of the frame; a task can overrun this. */
export const PAINT_BUDGET_MS = 6;

/** The canvas a task draws into says, as `drawPending`, whether a draw it was asked for has yet to run: a property, not an
 * attribute, so marking it costs no style work. The acceptance suite reads it to time a chart to its drawing. */
type Drawn = Element & { drawPending?: boolean };

/** The widget a task draws for, read from its element's place on the page once it is placed: null, none found. */
function widgetOf(state: { element: Drawn | null; widget?: string }): string | null {
  if (state.widget !== undefined) return state.widget;
  const widget = state.element?.closest<HTMLElement>('[data-widget]')?.dataset.widget;
  if (widget !== undefined) state.widget = widget;
  return widget ?? null;
}

interface TaskState { visible: boolean; awake: boolean; woken: boolean; pending: boolean; element: Drawn | null; widget?: string }

export class FrameScheduler {
  /** `woken` records a wake that arrives while the task is drawing, so it is not lost. */
  private readonly tasks = new Map<FrameTask, TaskState>();
  /** Tasks with a draw asked for and not yet run, and the page's count of them as in flight while there are any. */
  private pendingDraws = 0;
  private drawing: (() => void) | null = null;
  private handle: number | null = null;
  private last: number | null = null;
  /** The first task the last frame's budget left waiting. */
  private next: FrameTask | null = null;
  private readonly host: FrameHost;

  constructor(host: FrameHost) { this.host = host; }

  /** Adds a task, awake and visible, drawing into `element`; the returned function removes it. */
  add(task: FrameTask, element: Element | null = null): () => void {
    const state: TaskState = { visible: true, awake: true, woken: false, pending: false, element: element as Drawn | null };
    this.tasks.set(task, state);
    this.setPending(state, true);
    this.schedule();
    return () => { this.setPending(state, false); this.tasks.delete(task); };
  }

  /** Whether `state` has a draw asked for and not yet run: on its canvas for the suite, and in the page's count. */
  private setPending(state: TaskState, pending: boolean): void {
    if (state.element) state.element.drawPending = pending;
    if (state.pending === pending) return;
    state.pending = pending;
    this.pendingDraws += pending ? 1 : -1;
    if (pending && this.pendingDraws === 1) this.drawing = busy('draws');
    else if (!pending && this.pendingDraws === 0) { this.drawing?.(); this.drawing = null; }
  }

  /** Asks for a frame: data, size, theme or pointer changed. */
  wake(task: FrameTask): void {
    const state = this.tasks.get(task);
    if (!state) return;
    state.awake = true;
    state.woken = true;
    if (state.visible) this.setPending(state, true);
    this.schedule();
  }

  /** Off-screen or hidden tasks keep their place but never draw. */
  setVisible(task: FrameTask, visible: boolean): void {
    const state = this.tasks.get(task);
    if (!state || state.visible === visible) return;
    state.visible = visible;
    this.setPending(state, visible);
    if (visible) { state.awake = true; this.schedule(); }
  }

  get running(): boolean { return this.handle != null; }

  private hasRunnableTask(): boolean {
    for (const state of this.tasks.values()) {
      if (state.awake && state.visible) return true;
    }
    return false;
  }

  private schedule(): void {
    if (this.handle != null || !this.hasRunnableTask()) return;
    this.handle = this.host.request(now => this.tick(now));
  }

  private tick(now: number): void {
    this.handle = null;
    const dt = this.last == null ? 16 : Math.min(MAX_DT, Math.max(0, now - this.last));
    this.last = now;
    const clock = this.host.clock ?? (() => performance.now());
    const started = clock();
    let ran = 0;
    // Round-robin: the tasks the last frame left waiting go first.
    const order = [...this.tasks.keys()];
    const first = this.next ? order.indexOf(this.next) : -1;
    this.next = null;
    const turn = first > 0 ? [...order.slice(first), ...order.slice(0, first)] : order;
    for (const task of turn) {
      const state = this.tasks.get(task);
      if (!state?.awake || !state.visible) continue;
      if (ran > 0 && clock() - started >= PAINT_BUDGET_MS) { this.next ??= task; continue; }
      ran++;
      state.woken = false;
      let again = false;
      const info = { now, dt, deadline: started + PAINT_BUDGET_MS };
      try {
        if (perfLog) timePaint(widgetOf(state), now, () => { again = task.frame(info); });
        else again = task.frame(info);
      } catch (error) { console.error(error); }
      state.awake = again || state.woken;
      if (!state.woken) this.setPending(state, false);
    }
    if (this.hasRunnableTask()) this.schedule();
    else this.last = null;
  }
}

/** The dashboard's shared scheduler: its tasks draw in the engine's paint phase. */
export const scheduler = new FrameScheduler({ request: callback => { engine.next('paint', frame => callback(frame.now)); return 0; } });
