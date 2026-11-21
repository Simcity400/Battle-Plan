import { engine } from './frame.ts';
import type { FrameEngine } from './frame.ts';
import { cssEase, reducedMotion, rowEase } from './glide.ts';

export const ENTRY_MS = 700;
export const EXIT_MS = 450;

export interface FadingRow { style: { opacity: string; setProperty(name: string, value: string): void; removeProperty(name: string): string } }

interface Fade { started: number; from: number; kind: 'entering' | 'leaving' | 'returning'; now: number }

export class RowFades {
  private readonly fading = new Map<FadingRow, Fade>();
  private task: ReturnType<FrameEngine['every']> | null = null;
  private readonly frames: FrameEngine;
  private readonly clock: () => number;
  private readonly reduced: () => boolean;

  constructor(frames: FrameEngine = engine, clock: () => number = () => performance.now(), reduced: () => boolean = reducedMotion) {
    this.frames = frames;
    this.clock = clock;
    this.reduced = reduced;
  }

  enter(row: FadingRow): void {
    if (this.reduced()) return;
    this.fading.set(row, { started: this.clock(), from: 0, kind: 'entering', now: 0 });
    this.write(row, 0, 1);
    this.run();
  }

  leave(row: FadingRow): void {
    const held = this.fading.get(row);
    if (held?.kind === 'leaving') return;
    if (this.reduced()) { this.clear(row); return; }
    const from = held ? held.now : 1;
    this.fading.set(row, { started: this.clock(), from, kind: 'leaving', now: from });
    row.style.removeProperty('--row-flash-opacity');
    row.style.opacity = String(from);
    this.run();
  }

  stay(row: FadingRow): void {
    const held = this.fading.get(row);
    if (held?.kind !== 'leaving') return;
    this.fading.set(row, { started: this.clock(), from: held.now, kind: 'returning', now: held.now });
    this.run();
  }

  has(row: FadingRow): boolean { return this.fading.has(row); }

  private run(): void {
    if (this.task) this.task.wake();
    else this.task = this.frames.every('paint', frame => this.step(frame.now));
  }

  private write(row: FadingRow, opacity: number, flash: number): void {
    row.style.opacity = String(opacity);
    row.style.setProperty('--row-flash-opacity', String(flash));
  }

  private clear(row: FadingRow): void {
    this.fading.delete(row);
    row.style.opacity = '';
    row.style.removeProperty('--row-flash-opacity');
  }

  private step(now: number): boolean {
    for (const [row, fade] of this.fading) {
      if (fade.kind !== 'entering') {
        const t = (now - fade.started) / EXIT_MS;
        if (t >= 1) { this.clear(row); continue; }
        const e = cssEase(Math.max(0, t));
        fade.now = fade.kind === 'leaving' ? fade.from * (1 - e) : fade.from + (1 - fade.from) * e;
        row.style.opacity = String(fade.now);
        continue;
      }
      const t = (now - fade.started) / ENTRY_MS;
      if (t >= 1) { this.clear(row); continue; }
      fade.now = rowEase(Math.max(0, t));
      this.write(row, fade.now, 1 - fade.now);
    }
    return this.fading.size > 0;
  }
}

export const rowFades = new RowFades();
