import { startTransition, useEffect, useRef, useState } from 'react';
import type { HourWindow } from './model';
import { perfLog } from '../shell/perf.ts';

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
  bounds: HourWindow | null;
}

/** One wall clock per dashboard. React only subscribes where time is needed. */
export class PlaybackController {
  private snapshot: PlaybackSnapshot = { position: 0, hour: 0, fraction: 0, playing: false, hoursPerSecond: 1, sequence: 0, seekSequence: 0, bounds: null };
  private listeners = new Set<() => void>();
  private hourListeners = new Set<() => void>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private frame: number | null = null;
  private lastTime = 0;

  getSnapshot = () => this.snapshot;
  /** The position at `now`, carried forward from the last tick while playing so
   * animations move smoothly between ticks. Never past the branch end. */
  positionAt = (now: number = performance.now()) => {
    const { playing, position, hoursPerSecond, bounds } = this.snapshot;
    if (!playing || !bounds) return position;
    return Math.min(bounds.lastHour, position + Math.max(0, now - this.lastTime) / 1000 * hoursPerSecond);
  };
  getHour = () => this.snapshot.hour;
  getMotion = () => this.snapshot.playing ? this.snapshot.hoursPerSecond : undefined;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  subscribeHour = (listener: () => void) => { this.hourListeners.add(listener); return () => { this.hourListeners.delete(listener); }; };

  setBounds(bounds: HourWindow | null, reset = false): void {
    const previous = this.snapshot.bounds;
    if (!reset && previous?.firstHour === bounds?.firstHour && previous?.lastHour === bounds?.lastHour) return;
    const position = !bounds ? 0 : reset || !previous ? bounds.firstHour
      : Math.max(bounds.firstHour, Math.min(bounds.lastHour, this.snapshot.position));
    this.publish({ ...this.snapshot, bounds, position, playing: false }, true);
    this.syncTimer();
  }

  seek(position: number): void {
    const bounds = this.snapshot.bounds;
    if (!bounds || !Number.isFinite(position)) return;
    this.publish({ ...this.snapshot, position: Math.max(bounds.firstHour, Math.min(bounds.lastHour, Math.floor(position))), playing: false }, true);
    this.syncTimer();
  }

  /** Forward steps keep presentation history; a step back starts it again. */
  step(direction: -1 | 1): void {
    const bounds = this.snapshot.bounds;
    if (!bounds) return;
    const position = Math.max(bounds.firstHour, Math.min(bounds.lastHour, this.snapshot.hour + direction));
    this.publish({ ...this.snapshot, position, playing: false }, direction < 0);
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
    if (this.timer != null) clearInterval(this.timer);
    if (this.frame != null) cancelAnimationFrame(this.frame);
    this.timer = null; this.frame = null;
  }

  /** One playback step: the time since the last step, at the current speed. */
  private advance(now: number): void {
    const seconds = Math.max(0, now - this.lastTime) / 1000;
    this.lastTime = now;
    const bound = this.snapshot.bounds?.lastHour ?? this.snapshot.position;
    const position = Math.min(bound, this.snapshot.position + seconds * this.snapshot.hoursPerSecond);
    this.publish({ ...this.snapshot, position, playing: position < bound });
    if (!this.snapshot.playing) this.syncTimer();
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

  private publish(next: PlaybackSnapshot, discontinuity = false): void {
    const hour = Math.floor(next.position);
    const hourChanged = hour !== this.snapshot.hour;
    this.snapshot = { ...next, hour, fraction: next.position - hour,
      sequence: this.snapshot.sequence + (hourChanged || discontinuity ? 1 : 0),
      seekSequence: this.snapshot.seekSequence + (discontinuity ? 1 : 0) };
    perfLog?.clock(this.snapshot.position, this.snapshot.playing, this.snapshot.hoursPerSecond, this.snapshot.sequence, this.snapshot.seekSequence);
    for (const listener of this.listeners) listener();
    if (hourChanged || discontinuity) for (const listener of this.hourListeners) listener();
  }

  private syncTimer(): void {
    this.cancelTimer();
    this.lastTime = performance.now();
    if (!this.snapshot.playing) return;
    // Step once per display frame, so the playhead moves as smoothly as the screen refreshes; at 1,440 h/s a
    // 50 ms tick moved it 72 hours at a time. Without frames (tests, a hidden window) a timer keeps time instead.
    if (typeof requestAnimationFrame === 'function' && typeof document !== 'undefined' && document.visibilityState !== 'hidden') {
      const step = (now: number) => {
        this.frame = null;
        this.advance(Math.max(now, this.lastTime));
        if (this.snapshot.playing && this.frame == null && this.timer == null) this.frame = requestAnimationFrame(step);
      };
      this.frame = requestAnimationFrame(step);
    } else {
      this.timer = setInterval(() => this.advance(performance.now()), 50);
    }
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

/** The clock's hour; unfollowed (pinned) widgets read it without re-rendering for it. */
export function usePlaybackHour(clock: PlaybackController, follow = true): number {
  return useTransitionValue(follow ? clock.subscribeHour : null, clock.getHour);
}

/** The rate charts ease at: the playback speed while playing, none while paused, so a jump,
 * a scrub or a new selection is drawn at once. */
export function usePlaybackMotion(clock: PlaybackController): number | undefined {
  return useTransitionValue(clock.subscribe, clock.getMotion);
}
