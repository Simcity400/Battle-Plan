import { useSyncExternalStore } from 'react';
import type { HourWindow } from './model';

// UI limits retained until end-to-end playback throughput has been measured.
export const MIN_PLAYBACK_SPEED = 0.25;
export const MAX_PLAYBACK_SPEED = 24;
export const PLAYBACK_SPEEDS = [0.25, 1, 4, 12, 24] as const;
export const validPlaybackSpeed = (speed: number) => Number.isFinite(speed)
  && speed >= MIN_PLAYBACK_SPEED && speed <= MAX_PLAYBACK_SPEED;

export interface PlaybackSnapshot {
  position: number;
  hour: number;
  fraction: number;
  playing: boolean;
  hoursPerSecond: number;
  sequence: number;
  bounds: HourWindow | null;
}

/** One wall clock per dashboard. React only subscribes where time is needed. */
export class PlaybackController {
  private snapshot: PlaybackSnapshot = { position: 0, hour: 0, fraction: 0, playing: false, hoursPerSecond: 1, sequence: 0, bounds: null };
  private listeners = new Set<() => void>();
  private hourListeners = new Set<() => void>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private lastTime = 0;

  getSnapshot = () => this.snapshot;
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

  setPlaying(playing: boolean): void {
    if (!this.snapshot.bounds) return;
    this.publish({ ...this.snapshot, playing: playing && this.snapshot.position < this.snapshot.bounds.lastHour });
    this.syncTimer();
  }

  setSpeed(hoursPerSecond: number): void {
    if (!validPlaybackSpeed(hoursPerSecond)) return;
    this.publish({ ...this.snapshot, hoursPerSecond });
    this.lastTime = performance.now();
  }

  stop(): void {
    if (this.timer != null) clearInterval(this.timer);
    this.timer = null;
    this.publish({ ...this.snapshot, playing: false });
  }

  private publish(next: PlaybackSnapshot, discontinuity = false): void {
    const hour = Math.floor(next.position);
    const hourChanged = hour !== this.snapshot.hour;
    this.snapshot = { ...next, hour, fraction: next.position - hour,
      sequence: this.snapshot.sequence + (hourChanged || discontinuity ? 1 : 0) };
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
