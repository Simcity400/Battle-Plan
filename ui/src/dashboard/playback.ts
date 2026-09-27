import { useSyncExternalStore } from 'react';
import type { HourWindow } from './model';

// The cap is about a month per second, measured end to end on a recording: the map
// follows only the latest hour and widgets look hours up locally, so a tick that
// skips many hours costs no more.
export const MIN_PLAYBACK_SPEED = 0.25;
export const MAX_PLAYBACK_SPEED = 720;
export const PLAYBACK_SPEEDS = [0.25, 1, 4, 24, 168, 720] as const;
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
    if (this.timer != null) clearInterval(this.timer);
    this.timer = null;
    this.publish({ ...this.snapshot, position: this.bank(), playing: false });
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
    for (const listener of this.listeners) listener();
    if (hourChanged || discontinuity) for (const listener of this.hourListeners) listener();
  }

  private syncTimer(): void {
    if (this.timer != null) clearInterval(this.timer);
    this.timer = null;
    this.lastTime = performance.now();
    if (!this.snapshot.playing) return;
    this.timer = setInterval(() => {
      const now = performance.now();
      const seconds = (now - this.lastTime) / 1000;
      this.lastTime = now;
      const bound = this.snapshot.bounds?.lastHour ?? this.snapshot.position;
      const position = Math.min(bound, this.snapshot.position + seconds * this.snapshot.hoursPerSecond);
      this.publish({ ...this.snapshot, position, playing: position < bound });
      if (!this.snapshot.playing) this.syncTimer();
    }, 50);
  }
}

const noSubscription = () => () => {};
export function usePlaybackHour(clock: PlaybackController, follow = true): number {
  return useSyncExternalStore(follow ? clock.subscribeHour : noSubscription, clock.getHour, clock.getHour);
}
