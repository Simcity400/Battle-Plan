import { flushSync } from 'react-dom';
import { engine } from '../engine/frame.ts';
import type { FrameEngine } from '../engine/frame';

type GeometryRead = () => (() => void);

export class GeometryReads {
  private readonly pending = new Set<GeometryRead>();
  private queued = false;
  private readonly frames: Pick<FrameEngine, 'next' | 'frame'>;
  private readonly commit: (writes: () => void) => void;
  private readonly soon: (run: () => void) => void;

  constructor(frames: Pick<FrameEngine, 'next' | 'frame'>, commit: (writes: () => void) => void, soon: (run: () => void) => void = run => queueMicrotask(run)) {
    this.frames = frames; this.commit = commit; this.soon = soon;
  }

  invalidate(read: GeometryRead): void {
    this.pending.add(read);
    if (this.queued) return;
    this.queued = true;
    if (this.frames.frame) this.frames.next('after', () => this.read());
    else this.soon(() => this.read());
  }

  cancel(read: GeometryRead): void { this.pending.delete(read); }

  private read(): void {
    this.queued = false;
    const reads = [...this.pending];
    this.pending.clear();
    const writes: (() => void)[] = [], errors: unknown[] = [];
    for (const read of reads) { try { writes.push(read()); } catch (error) { errors.push(error); } }
    this.commit(() => { for (const write of writes) { try { write(); } catch (error) { errors.push(error); } } });
    for (const error of errors) reportError(error);
  }
}

const page = new GeometryReads(engine, flushSync);

export function invalidateGeometry(read: GeometryRead): void { page.invalidate(read); }

export function cancelGeometry(read: GeometryRead): void { page.cancel(read); }
