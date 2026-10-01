/** One animation loop for every chart on the dashboard. A task draws when it is
 * woken and keeps frames coming only while it says it is still moving; hidden
 * tasks never draw, and the loop stops entirely when nothing is moving. */

export interface FrameInfo { now: number; dt: number }
export interface FrameTask {
  /** Draws one frame; returns true to be called again next frame. */
  frame(info: FrameInfo): boolean;
}
export interface FrameHost {
  request(callback: (now: number) => void): number;
}

/** Longest step reported to tasks, so a tab returning from the background doesn't jump. */
const MAX_DT = 64;

export class FrameScheduler {
  /** `woken` records a wake that arrives while the task is drawing, so it is not lost. */
  private readonly tasks = new Map<FrameTask, { visible: boolean; awake: boolean; woken: boolean }>();
  private handle: number | null = null;
  private last: number | null = null;
  private readonly host: FrameHost;

  constructor(host: FrameHost) { this.host = host; }

  /** Adds a task, awake and visible; the returned function removes it. */
  add(task: FrameTask): () => void {
    this.tasks.set(task, { visible: true, awake: true, woken: false });
    this.schedule();
    return () => { this.tasks.delete(task); };
  }

  /** Asks for a frame: data, size, theme or pointer changed. */
  wake(task: FrameTask): void {
    const state = this.tasks.get(task);
    if (!state) return;
    state.awake = true;
    state.woken = true;
    this.schedule();
  }

  /** Off-screen or hidden tasks keep their place but never draw. */
  setVisible(task: FrameTask, visible: boolean): void {
    const state = this.tasks.get(task);
    if (!state || state.visible === visible) return;
    state.visible = visible;
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
    for (const [task, state] of this.tasks) {
      if (!state.awake || !state.visible) continue;
      state.woken = false;
      let again = false;
      try { again = task.frame({ now, dt }); } catch (error) { console.error(error); }
      state.awake = again || state.woken;
    }
    if (this.hasRunnableTask()) this.schedule();
    else this.last = null;
  }
}

/** The dashboard's shared scheduler. */
export const scheduler = new FrameScheduler(typeof requestAnimationFrame === 'function'
  ? { request: callback => requestAnimationFrame(callback) }
  : { request: callback => setTimeout(() => callback(Date.now()), 16) as unknown as number });
