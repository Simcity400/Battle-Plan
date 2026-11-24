import { engine } from './frame.ts';
import type { FrameEngine } from './frame.ts';

export function bezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const along = (s: number, p1: number, p2: number) => 3 * (1 - s) * (1 - s) * s * p1 + 3 * (1 - s) * s * s * p2 + s * s * s;
  const slope = (s: number) => 3 * (1 - s) * (1 - s) * x1 + 6 * (1 - s) * s * (x2 - x1) + 3 * s * s * (1 - x2);
  return t => {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    let s = t;
    for (let step = 0; step < 8; step++) {
      const d = slope(s);
      if (Math.abs(d) < 1e-9) break;
      s = Math.min(1, Math.max(0, s - (along(s, x1, x2) - t) / d));
    }
    return along(s, y1, y2);
  };
}

export const cssEase = bezier(0.25, 0.1, 0.25, 1);
export const rowEase = bezier(0.2, 0.8, 0.2, 1);

interface Gliding { from: number; to: number; started: number; ms: number; now: number }

export interface GlidedStyle { transform?: string }

export class StyleGlides {
  private readonly gliding = new Map<GlidedStyle, Gliding>();
  private readonly shown = new WeakMap<GlidedStyle, number>();
  private task: ReturnType<FrameEngine['every']> | null = null;
  private readonly format: (value: number) => string;
  private readonly curve: (t: number) => number;
  private readonly frames: FrameEngine;
  private readonly clock: () => number;

  constructor(format: (value: number) => string, curve: (t: number) => number, frames: FrameEngine = engine, clock: () => number = () => performance.now()) {
    this.format = format;
    this.curve = curve;
    this.frames = frames;
    this.clock = clock;
  }

  to(style: GlidedStyle, value: number, ms: number): void {
    const held = this.gliding.get(style);
    if (held ? held.to === value : this.shown.get(style) === value) return;
    const at = held ? held.now : this.shown.get(style);
    if (!(ms > 0) || at === undefined) {
      this.gliding.delete(style);
      this.put(style, value);
      return;
    }
    this.gliding.set(style, { from: at, to: value, started: this.clock(), ms, now: at });
    if (this.task) this.task.wake();
    else this.task = this.frames.every('paint', frame => this.step(frame.now));
  }

  at(style: GlidedStyle): number | undefined {
    return this.gliding.get(style)?.now ?? this.shown.get(style);
  }

  private put(style: GlidedStyle, value: number): void {
    this.shown.set(style, value);
    style.transform = this.format(value);
  }

  private step(now: number): boolean {
    for (const [style, glide] of this.gliding) {
      const t = Math.min(1, Math.max(0, (now - glide.started) / glide.ms));
      glide.now = glide.from + (glide.to - glide.from) * this.curve(t);
      this.put(style, t >= 1 ? glide.to : glide.now);
      if (t >= 1) this.gliding.delete(style);
    }
    return this.gliding.size > 0;
  }
}

export const barGlides = new StyleGlides(fraction => `scaleX(${fraction})`, cssEase);
export const rowGlides = new StyleGlides(y => `translateY(${y}px)`, rowEase);

export const reducedMotion = (): boolean => typeof document !== 'undefined' && document.documentElement.dataset.motion === 'reduced';
