/** One animation loop for every painted widget on the dashboard. A task draws when it is
 * woken and keeps frames coming only while it says it is still moving; hidden
 * tasks never draw, and the loop stops entirely when nothing is moving. Each frame
 * gives tasks a fixed budget, round-robin: a task past it waits for the next frame
 * (first in line there), so under load a widget paints a little less often and a
 * frame never overruns. */

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
/** What painting may take of a frame, leaving the rest of its 16.7 ms to style, layout and the compositor. */
export const PAINT_BUDGET_MS = 6;

/** The canvas a task draws into says, as `drawPending`, whether a draw it was asked for has yet to run: a property, not an
 * attribute, so marking it costs no style work. The acceptance suite reads it to time a chart to its drawing. */
type Drawn = Element & { drawPending?: boolean };

export class FrameScheduler {
  /** `woken` records a wake that arrives while the task is drawing, so it is not lost. */
  private readonly tasks = new Map<FrameTask, { visible: boolean; awake: boolean; woken: boolean; element: Drawn | null }>();
  private handle: number | null = null;
  private last: number | null = null;
  /** The first task the last frame's budget left waiting. */
  private next: FrameTask | null = null;
  private readonly host: FrameHost;

  constructor(host: FrameHost) { this.host = host; }

  /** Adds a task, awake and visible, drawing into `element`; the returned function removes it. */
  add(task: FrameTask, element: Element | null = null): () => void {
    const state = { visible: true, awake: true, woken: false, element: element as Drawn | null };
    this.tasks.set(task, state);
    if (state.element) state.element.drawPending = true;
    this.schedule();
    return () => { if (state.element) state.element.drawPending = false; this.tasks.delete(task); };
  }

  /** Asks for a frame: data, size, theme or pointer changed. */
  wake(task: FrameTask): void {
    const state = this.tasks.get(task);
    if (!state) return;
    state.awake = true;
    state.woken = true;
    if (state.element && state.visible) state.element.drawPending = true;
    this.schedule();
  }

  /** Off-screen or hidden tasks keep their place but never draw. */
  setVisible(task: FrameTask, visible: boolean): void {
    const state = this.tasks.get(task);
    if (!state || state.visible === visible) return;
    state.visible = visible;
    if (state.element) state.element.drawPending = visible;
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
      try { again = task.frame({ now, dt, deadline: started + PAINT_BUDGET_MS }); } catch (error) { console.error(error); }
      state.awake = again || state.woken;
      if (state.element && !state.woken) state.element.drawPending = false;
    }
    if (this.hasRunnableTask()) this.schedule();
    else this.last = null;
  }
}

/** The dashboard's shared scheduler. */
export const scheduler = new FrameScheduler(typeof requestAnimationFrame === 'function'
  ? { request: callback => requestAnimationFrame(callback) }
  : { request: callback => setTimeout(() => callback(Date.now()), 16) as unknown as number });
