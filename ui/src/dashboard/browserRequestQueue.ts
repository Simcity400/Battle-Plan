export type BrowserRequestLane = 'history' | 'interactive' | 'control';

const LIMITS: Readonly<Record<BrowserRequestLane, number>> = { history: 3, interactive: 2, control: 1 };
const aborted = () => new DOMException('Request cancelled', 'AbortError');

interface Task {
  lane: BrowserRequestLane;
  sourceId?: string;
  cancel: () => void;
  start: () => void;
}

/** Reserve HTTP connections for dated facts and release commands while histories load.
 * Cancelled active work keeps its connection until fetch and its body settle. */
export class BrowserRequestQueue {
  private readonly waiting: Record<BrowserRequestLane, Task[]> = { history: [], interactive: [], control: [] };
  private readonly running = new Set<Task>();
  private readonly counts: Record<BrowserRequestLane, number> = { history: 0, interactive: 0, control: 0 };

  run<T>(lane: BrowserRequestLane, operation: (signal: AbortSignal) => Promise<T>,
    options: { signal?: AbortSignal; sourceId?: string; onCancelledValue?: (value: T) => Promise<void> } = {}): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      if (options.signal?.aborted) { reject(aborted()); return; }
      const controller = new AbortController();
      let started = false;
      const detach = () => options.signal?.removeEventListener('abort', task.cancel);
      const task: Task = {
        lane, sourceId: options.sourceId,
        cancel: () => {
          if (controller.signal.aborted) return;
          controller.abort();
          // An active request owns its slot until its network work ends.
          if (started) return;
          const at = this.waiting[lane].indexOf(task);
          if (at >= 0) this.waiting[lane].splice(at, 1);
          detach();
          reject(aborted());
        },
        start: () => {
          started = true;
          this.running.add(task);
          this.counts[lane]++;
          void Promise.resolve().then(() => {
            if (controller.signal.aborted) throw aborted();
            return operation(controller.signal);
          }).then(async value => {
            if (controller.signal.aborted) {
              // A cancelled open can still return a lease; let its owner release it.
              try { await options.onCancelledValue?.(value); } catch { /* Cancellation remains the result if late cleanup fails. */ }
              reject(aborted());
            }
            else resolve(value);
          }, error => reject(controller.signal.aborted ? aborted() : error)).finally(() => {
            detach();
            this.running.delete(task);
            this.counts[lane]--;
            this.pump(lane);
          });
        },
      };
      options.signal?.addEventListener('abort', task.cancel, { once: true });
      this.waiting[lane].push(task);
      this.pump(lane);
    });
  }

  /** Cancel only the released lease's work; other recordings keep their requests. */
  cancelSource(sourceId: string): void {
    for (const tasks of Object.values(this.waiting)) for (const task of [...tasks]) if (task.sourceId === sourceId) task.cancel();
    for (const task of this.running) if (task.sourceId === sourceId) task.cancel();
  }

  private pump(lane: BrowserRequestLane): void {
    while (this.counts[lane] < LIMITS[lane]) {
      const task = this.waiting[lane].shift();
      if (!task) break;
      task.start();
    }
  }
}
