import { perfLog, timePaint } from '../shell/perf.ts';
import { busy } from '../shell/busy.ts';
import { engine } from '../engine/frame.ts';

export interface FrameInfo {
  now: number;
  dt: number;
  deadline?: number;
}
export interface FrameTask {
  frame(info: FrameInfo): boolean;
}
export interface FrameHost {
  request(callback: (now: number) => void): number;
  clock?: () => number;
}

const MAX_DT = 64;
export const PAINT_BUDGET_MS = 6;

type Drawn = Element & { drawPending?: boolean };

function widgetOf(state: { element: Drawn | null; widget?: string }): string | null {
  if (state.widget !== undefined) return state.widget;
  const widget = state.element?.closest<HTMLElement>('[data-widget]')?.dataset.widget;
  if (widget !== undefined) state.widget = widget;
  return widget ?? null;
}

interface TaskState { visible: boolean; awake: boolean; woken: boolean; syncPending: boolean; asyncPending: boolean; pending: boolean; element: Drawn | null; widget?: string }

export class FrameScheduler {
  private readonly tasks = new Map<FrameTask, TaskState>();
  private pendingDraws = 0;
  private drawing: (() => void) | null = null;
  private handle: number | null = null;
  private last: number | null = null;
  private next: FrameTask | null = null;
  private readonly host: FrameHost;

  constructor(host: FrameHost) { this.host = host; }

  add(task: FrameTask, element: Element | null = null): () => void {
    const state: TaskState = { visible: true, awake: true, woken: false, syncPending: true, asyncPending: false, pending: false, element: element as Drawn | null };
    this.tasks.set(task, state);
    this.refreshPending(state);
    this.schedule();
    return () => { state.syncPending = false; state.asyncPending = false; this.refreshPending(state); this.tasks.delete(task); };
  }

  private refreshPending(state: TaskState): void {
    const pending = state.visible && (state.syncPending || state.asyncPending);
    if (state.element) state.element.drawPending = pending;
    if (state.pending === pending) return;
    state.pending = pending;
    this.pendingDraws += pending ? 1 : -1;
    if (pending && this.pendingDraws === 1) this.drawing = busy('draws');
    else if (!pending && this.pendingDraws === 0) { this.drawing?.(); this.drawing = null; }
  }

  wake(task: FrameTask): void {
    const state = this.tasks.get(task);
    if (!state) return;
    state.awake = true;
    state.woken = true;
    state.syncPending = true;
    this.refreshPending(state);
    this.schedule();
  }

  setAsyncPending(task: FrameTask, pending: boolean): void {
    const state = this.tasks.get(task);
    if (!state || state.asyncPending === pending) return;
    state.asyncPending = pending;
    this.refreshPending(state);
  }

  setVisible(task: FrameTask, visible: boolean): void {
    const state = this.tasks.get(task);
    if (!state || state.visible === visible) return;
    state.visible = visible;
    state.syncPending = visible;
    this.refreshPending(state);
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
      if (!state.woken) { state.syncPending = false; this.refreshPending(state); }
    }
    if (this.hasRunnableTask()) this.schedule();
    else this.last = null;
  }
}

export const scheduler = new FrameScheduler({ request: callback => { engine.next('paint', frame => callback(frame.now)); return 0; } });
