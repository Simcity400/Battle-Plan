
export const PHASES = ['clock', 'data', 'measure', 'paint', 'commit', 'after'] as const;
export type Phase = typeof PHASES[number];

export interface EngineFrame {
  readonly now: number;
  readonly dt: number;
  readonly index: number;
  readonly phase: Phase;
}

export interface FrameHost {
  request(run: (now: number) => void): void;
}

export interface EngineTask {
  wake(): void;
  stop(): void;
}

const MAX_DT = 64;
const FALLBACK_MS = 16;

interface Task { phase: number; run: (frame: EngineFrame) => boolean | void; awake: boolean; stopped: boolean }

export class FrameEngine {
  private readonly host: FrameHost;
  private readonly once: ((frame: EngineFrame) => void)[][] = PHASES.map(() => []);
  private readonly tasks: Set<Task>[] = PHASES.map(() => new Set());
  private asked = false;
  private last: number | null = null;
  private index = 0;
  private running = -1;
  private current: EngineFrame | null = null;

  constructor(host: FrameHost) { this.host = host; }

  get frame(): EngineFrame | null { return this.current; }

  next(phase: Phase, run: (frame: EngineFrame) => void): () => void {
    const list = this.once[PHASES.indexOf(phase)];
    list.push(run);
    this.schedule();
    return () => { const at = list.indexOf(run); if (at >= 0) list.splice(at, 1); };
  }

  every(phase: Phase, run: (frame: EngineFrame) => boolean | void): EngineTask {
    const task: Task = { phase: PHASES.indexOf(phase), run, awake: true, stopped: false };
    this.tasks[task.phase].add(task);
    this.schedule();
    return {
      wake: () => { if (task.stopped) return; task.awake = true; this.schedule(); },
      stop: () => { task.stopped = true; this.tasks[task.phase].delete(task); },
    };
  }

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

  get busy(): boolean { return this.asked || this.running >= 0; }
}

const pageHost: FrameHost = {
  request: run => {
    if (typeof requestAnimationFrame === 'function' && !(typeof document !== 'undefined' && document.visibilityState === 'hidden')) requestAnimationFrame(run);
    else setTimeout(() => run(performance.now()), FALLBACK_MS);
  },
};

export const engine = new FrameEngine(pageHost);
