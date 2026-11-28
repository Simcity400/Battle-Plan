export interface MotionClock {
  readonly playing: boolean;
  readonly anchorPosition: number;
  readonly anchorTimeMs: number;
  readonly speed: number;
  readonly seek: number;
  readonly exactHour: number;
  readonly lastHour?: number;
}

export function positionAt(clock: MotionClock, wallNowMs: number): number {
  if (!clock.playing || !(clock.speed > 0)) return clock.anchorPosition;
  const elapsedS = Math.max(0, wallNowMs - clock.anchorTimeMs) / 1000;
  const raw = clock.anchorPosition + elapsedS * clock.speed;
  return clock.lastHour !== undefined ? Math.min(clock.lastHour, raw) : raw;
}
