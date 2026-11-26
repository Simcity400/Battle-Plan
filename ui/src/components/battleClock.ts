import type { PlaybackController } from '../dashboard/playback.ts';
import { pageClock } from '../dashboard/widgetClock.ts';
import { engine } from '../engine/frame.ts';

export interface BattleSpan {
  readonly first: number;
  readonly last: number;
}

export const PLAY_SECONDS = 12;
const SLOWEST = 1;
const FASTEST = 24;

export function playRate(span: BattleSpan): number {
  return Math.min(FASTEST, Math.max(SLOWEST, (span.last - span.first) / PLAY_SECONDS));
}

let issued = 0;

export function issueSequence(): number { return ++issued; }

export interface BattleClockHost {
  readonly now: () => number;
  readonly next: (run: () => void) => () => void;
}

const pageHost: BattleClockHost = { now: () => performance.now(), next: run => engine.next('clock', run) };

export class BattleClock {
  readonly rate: number;
  private position: number;
  private anchor = 0;
  private running = false;
  private shownHour: number;
  private seeks = 0;
  private jumps = 0;
  private from: number;
  private moved = 0;
  private seen: number | null = null;
  private changes = issueSequence();
  private frame: (() => void) | null = null;
  private listeners = new Set<() => void>();
  private frameListeners = new Set<() => void>();

  readonly span: BattleSpan;
  private readonly host: BattleClockHost;

  constructor(span: BattleSpan, start: number, host: BattleClockHost = pageHost) {
    this.span = span;
    this.host = host;
    this.rate = playRate(span);
    this.position = this.clamp(start);
    this.shownHour = Math.floor(this.position);
    this.from = this.shownHour;
  }

  get playing(): boolean { return this.running; }
  get sequence(): number { return this.changes; }
  get seekSequence(): number { return this.seeks; }
  get jumpSequence(): number { return this.jumps; }
  get playedFrom(): number { return this.from; }
  get movedSequence(): number { return this.moved; }
  get seenThrough(): number | null { return this.seen; }
  get resumeFrom(): number { return this.seen === null ? this.from : Math.max(this.from, this.seen); }

  acknowledge(hour: number, sequence: number): void {
    if (sequence >= this.moved && hour <= this.positionAt()) this.seen = Math.max(this.seen ?? hour, hour);
  }

  getHour = (): number => this.shownHour;
  getPlaying = (): boolean => this.running;

  positionAt(now: number = this.host.now()): number {
    return this.running ? this.clamp(this.position + (now - this.anchor) / 1000 * this.rate) : this.position;
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  subscribeFrame = (listener: () => void): (() => void) => {
    this.frameListeners.add(listener);
    return () => { this.frameListeners.delete(listener); };
  };

  play(): void {
    if (this.running || this.span.last <= this.span.first) return;
    if (this.position >= this.span.last) { this.position = this.span.first; this.seeks++; this.jumps++; this.from = this.span.first; this.moved = issueSequence(); this.seen = null; }
    this.anchor = this.host.now();
    this.running = true;
    this.changed();
    this.schedule();
  }

  pause(): void {
    if (!this.running) return;
    this.position = this.positionAt();
    this.running = false;
    this.changed();
  }

  toggle(): void { if (this.running) this.pause(); else this.play(); }

  seek(position: number): void {
    const next = this.clamp(position);
    const current = this.positionAt();
    if (next === current) return;
    this.position = next;
    this.anchor = this.host.now();
    this.jumps++;
    this.from = Math.floor(next);
    this.moved = issueSequence();
    this.seen = null;
    if (next < current) this.seeks++;
    this.changed();
  }

  step(hours: number): void { this.seek(Math.floor(this.positionAt()) + hours); }

  dispose(): void {
    this.running = false;
    this.frame?.();
    this.frame = null;
    this.listeners.clear();
    this.frameListeners.clear();
  }

  private clamp(position: number): number {
    return Math.min(this.span.last, Math.max(this.span.first, position));
  }

  private changed(): void {
    this.changes = issueSequence();
    this.shownHour = Math.floor(this.positionAt());
    for (const listener of this.listeners) listener();
    for (const listener of this.frameListeners) listener();
  }

  private schedule(): void {
    if (this.frame) return;
    this.frame = this.host.next(this.tick);
  }

  private tick = (): void => {
    this.frame = null;
    if (!this.running) return;
    const now = this.host.now();
    const position = this.positionAt(now);
    if (position >= this.span.last) {
      this.position = this.span.last;
      this.running = false;
      this.changed();
      return;
    }
    const hour = Math.floor(position);
    if (hour !== this.shownHour) {
      this.shownHour = hour;
      for (const listener of this.listeners) listener();
    }
    for (const listener of this.frameListeners) listener();
    this.schedule();
  };
}

const pausing = new WeakMap<PlaybackController, { holds: number; halts: number }>();

export function sharedPause(clock: PlaybackController): { isPlaying(): boolean; pause(): void; play(): void } {
  const page = pageClock(clock);
  return {
    isPlaying: () => page.getSnapshot().playing || pausing.has(page),
    pause: () => {
      const held = pausing.get(page);
      if (held) {
        held.holds += 1;
        return;
      }
      page.setPlaying(false);
      pausing.set(page, { holds: 1, halts: page.halted });
    },
    play: () => {
      const held = pausing.get(page);
      if (!held || --held.holds > 0) return;
      pausing.delete(page);
      if (page.halted === held.halts) page.setPlaying(true);
    },
  };
}
