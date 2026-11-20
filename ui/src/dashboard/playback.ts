import { startTransition, useEffect, useRef, useState } from 'react';
import type { HourWindow } from './model';
import { OUTSIDE, perfLog } from '../shell/perf.ts';
import { engine } from '../engine/frame.ts';
import type { EngineTask, FrameEngine } from '../engine/frame.ts';

// Widgets look hours up locally and the native map advances the same clock
// between host updates; neither clock waits for hour builders.
export const MIN_PLAYBACK_SPEED = 0.25;
export const MAX_PLAYBACK_SPEED = 1440;
export const PLAYBACK_SPEEDS = [0.25, 1, 4, 24, 168, 720, 1440] as const;
export const validPlaybackSpeed = (speed: number) => Number.isFinite(speed)
  && speed >= MIN_PLAYBACK_SPEED && speed <= MAX_PLAYBACK_SPEED;

/** A rate as a number and a unit: whole weeks or days per second where they fit,
 * else hours rounded for display (the clock keeps the exact rate). */
export function speedParts(hoursPerSecond: number): [string, string] {
  const unit = (count: number, name: string): [string, string] => [String(count), `${name}${count === 1 ? '' : 's'}/s`];
  if (hoursPerSecond >= 168 && hoursPerSecond % 168 === 0) return unit(hoursPerSecond / 168, 'week');
  if (hoursPerSecond >= 24 && hoursPerSecond % 24 === 0) return unit(hoursPerSecond / 24, 'day');
  return [String(Math.round(hoursPerSecond * 100) / 100), 'h/s'];
}
export const speedLabel = (hoursPerSecond: number) => speedParts(hoursPerSecond).join(' ');

export interface PlaybackSnapshot {
  position: number;
  hour: number;
  fraction: number;
  playing: boolean;
  hoursPerSecond: number;
  sequence: number;
  /** Distinguishes a seek from forward playback even when updates coalesce. */
  seekSequence: number;
  forwardSeek?: boolean;
  composedAt?: number;
  bounds: HourWindow | null;
}

/** One wall clock per dashboard, with two signals and no data tick:
 * - `subscribeFrame`: every published position, once per display frame while playing; for painters
 *   and for values read at the head that render only when they change (`useAtHead`).
 * - `subscribe`: the clock's anchor (play, pause, speed, seek, step, bounds, the branch end), not
 *   forward playback; the native maps run their own clock between anchors.
 * Fallback requests read the hour at the anchor. */
export class PlaybackController {
  private snapshot: PlaybackSnapshot = { position: 0, hour: 0, fraction: 0, playing: false, hoursPerSecond: 1, sequence: 0, seekSequence: 0, bounds: null };
  private listeners = new Set<() => void>();
  private anchorListeners = new Set<(previous: PlaybackSnapshot, next: PlaybackSnapshot) => void>();
  private discards = new Set<() => void>();
  private frameListeners = new Set<() => void>();
  private lastTime = 0;
  private framing = OUTSIDE;
  private readonly engine: FrameEngine;
  /** While playing: the step in the engine's clock phase, and the frame listeners told in its paint phase. */
  private stepTask: EngineTask | null = null;
  private paintTask: EngineTask | null = null;

  constructor(frames: FrameEngine = engine) { this.engine = frames; }

  /** The time of the animation frame whose step is being published, `OUTSIDE` any: what a measured run counts a frame
   * listener's painting toward. */
  get frameAt(): number { return this.framing; }

  getSnapshot = () => this.snapshot;
  /** The position at `now`, carried forward from the last tick while playing so
   * animations move smoothly between ticks. Never past the branch end. */
  positionAt = (now: number = performance.now()) => {
    const { playing, position, hoursPerSecond, bounds } = this.snapshot;
    if (!playing || !bounds) return position;
    return Math.min(bounds.lastHour, position + Math.max(0, now - this.lastTime) / 1000 * hoursPerSecond);
  };
  /** When the published position was taken, on the page's monotonic clock: what a native map
   * places an anchor composed from this snapshot by, so delivery delay adds no error. A clock
   * that is not playing stands still, so its position is current whenever it is read. */
  composedAt = () => this.snapshot.playing ? this.lastTime : performance.now();
  getHour = () => this.snapshot.hour;
  getMotion = () => this.snapshot.playing ? this.snapshot.hoursPerSecond : undefined;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  /** Called synchronously when a new playback anchor is published, before frame listeners paint it. */
  onAnchor = (listener: (previous: PlaybackSnapshot, next: PlaybackSnapshot) => void) => {
    this.anchorListeners.add(listener); return () => { this.anchorListeners.delete(listener); };
  };
  onDiscard = (listener: () => void) => { this.discards.add(listener); return () => { this.discards.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frameListeners.add(listener); return () => { this.frameListeners.delete(listener); }; };

  setBounds(bounds: HourWindow | null, reset = false): void {
    const previous = this.snapshot.bounds;
    if (!reset && previous?.firstHour === bounds?.firstHour && previous?.lastHour === bounds?.lastHour) return;
    const position = !bounds ? 0 : reset || !previous ? bounds.firstHour
      : Math.max(bounds.firstHour, Math.min(bounds.lastHour, this.snapshot.position));
    this.publish({ ...this.snapshot, bounds, position, playing: false, forwardSeek: false }, true);
    this.syncTimer();
  }

  seek(position: number): void {
    const bounds = this.snapshot.bounds;
    if (!bounds || !Number.isFinite(position)) return;
    const target = Math.max(bounds.firstHour, Math.min(bounds.lastHour, Math.floor(position)));
    this.publish({ ...this.snapshot, position: target, playing: false, forwardSeek: target > this.positionAt() }, true);
    this.syncTimer();
  }

  /** Forward steps keep presentation history; a step back starts it again. */
  step(direction: -1 | 1): void {
    const bounds = this.snapshot.bounds;
    if (!bounds) return;
    const current = this.bank();
    const position = Math.max(bounds.firstHour, Math.min(bounds.lastHour, Math.floor(current) + direction));
    this.publish({ ...this.snapshot, position, playing: false, forwardSeek: position > current }, position < current);
    this.syncTimer();
  }

  setPlaying(playing: boolean): void {
    if (!this.snapshot.bounds) return;
    // Keep the time played since the last tick, so pausing never steps back.
    const position = this.bank();
    this.publish({ ...this.snapshot, position, playing: playing && position < this.snapshot.bounds.lastHour });
    this.syncTimer();
  }

  setSpeed(hoursPerSecond: number): void {
    if (!validPlaybackSpeed(hoursPerSecond)) return;
    // Bank the time played at the old speed before switching.
    this.publish({ ...this.snapshot, position: this.bank(), hoursPerSecond });
  }

  stop(): void {
    this.cancelTimer();
    this.publish({ ...this.snapshot, position: this.bank(), playing: false });
  }

  private cancelTimer(): void {
    this.stepTask?.stop();
    this.paintTask?.stop();
    this.stepTask = null; this.paintTask = null;
  }

  /** One playback step in the engine's clock phase: the time since the last step, at the current speed. Its frame
   * listeners are told in the same frame's paint phase, once the frame's data has landed. */
  private advance(now: number): void {
    const seconds = Math.max(0, now - this.lastTime) / 1000;
    this.lastTime = now;
    const bound = this.snapshot.bounds?.lastHour ?? this.snapshot.position;
    const position = Math.min(bound, this.snapshot.position + seconds * this.snapshot.hoursPerSecond);
    this.publish({ ...this.snapshot, position, playing: position < bound }, false, true);
    if (!this.snapshot.playing) this.syncTimer();
  }

  private tellFrame(): void {
    for (const listener of this.frameListeners) listener();
  }

  /** The position now, with the clock's anchor moved to now. The anchor moves
   * before anyone is told, so a subscriber that stops or pauses the clock in
   * response never counts the same time twice. */
  private bank(): number {
    const now = performance.now();
    const position = this.positionAt(now);
    this.lastTime = now;
    return position;
  }

  /** `forward` marks a playback step, which moves no anchor unless it ends playback. */
  private publish(next: PlaybackSnapshot, discontinuity = false, forward = false): void {
    if (discontinuity && !next.forwardSeek) for (const discard of this.discards) discard();
    const previous = this.snapshot;
    const hour = Math.floor(next.position);
    const hourChanged = hour !== this.snapshot.hour;
    const anchor = !forward || next.playing !== this.snapshot.playing;
    this.snapshot = { ...next, hour, composedAt: next.playing ? this.lastTime : performance.now(), fraction: next.position - hour,
      sequence: this.snapshot.sequence + (hourChanged || discontinuity ? 1 : 0),
      seekSequence: this.snapshot.seekSequence + (discontinuity ? 1 : 0) };
    perfLog?.clock(this.snapshot.position, this.snapshot.playing, this.snapshot.hoursPerSecond, this.snapshot.sequence, this.snapshot.seekSequence, this.snapshot.composedAt);
    if (anchor) for (const listener of this.anchorListeners) listener(previous, this.snapshot);
    // A playing step's listeners paint in its frame's paint phase; anything else is told at once.
    if (!forward || !this.paintTask || !next.playing) this.tellFrame();
    else this.paintTask.wake();
    if (anchor) for (const listener of this.listeners) listener();
  }

  private syncTimer(): void {
    this.cancelTimer();
    if (!this.snapshot.playing) return;
    // One step per display frame, so the playhead moves as smoothly as the screen refreshes, in the page's one frame loop.
    this.paintTask = this.engine.every('paint', frame => {
      this.framing = frame.now;
      try { this.tellFrame(); } finally { this.framing = OUTSIDE; }
      return false;
    });
    this.stepTask = this.engine.every('clock', frame => {
      this.framing = frame.now;
      try { this.advance(Math.max(frame.now, this.lastTime)); } finally { this.framing = OUTSIDE; }
      return this.snapshot.playing;
    });
  }
}

/** A value of an outside store, as `read` returns it. A change reaches the component as a
 * transition: input answers at once, and every component renders the change in one commit,
 * in slices a newer change interrupts, each reading the value its own update carried. With
 * `subscribe` null the value is read as rendered and nothing re-renders for it. */
export function useTransitionValue<T>(subscribe: ((changed: () => void) => () => void) | null, read: () => T): T {
  // `shown` is the running subscription's (numbered by `generation`) only once that subscription
  // has set it; until then, and between subscriptions, the value is read. The first subscription
  // of a mount is the first numbered, so a value read at mount is already its own.
  const [shown, setShown] = useState(() => ({ read, value: read(), generation: subscribe ? 1 : 0 }));
  const generation = useRef(0);
  useEffect(() => {
    // Unsubscribed, nothing held is current: a later subscription reads until it sets its own.
    if (!subscribe) { generation.current++; return; }
    const mine = ++generation.current;
    const update = () => {
      const value = read();
      startTransition(() => setShown(previous => previous.read === read && previous.generation === mine && Object.is(previous.value, value)
        ? previous : { read, value, generation: mine }));
    };
    update();
    return subscribe(update);
  }, [subscribe, read]);
  return subscribe && shown.read === read && shown.generation === generation.current ? shown.value : read();
}


/** The rate charts ease at: the playback speed while playing, none while paused, so a jump,
 * a scrub or a new selection is drawn at once. Only anchors change it. */
export function usePlaybackMotion(clock: PlaybackController, active = true): number | undefined {
  return useTransitionValue(active ? clock.subscribe : null, clock.getMotion);
}
