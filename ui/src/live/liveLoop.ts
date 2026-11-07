import type { PlaybackController } from '../dashboard/playback';

/** Runs `paint` in every clock frame, at every anchor (a seek, a pause, a speed), and once
 * in the next frame after `wake` (an answer landed while paused, motion still settling). Painters
 * read local data and touch only what changed: a frame with nothing new costs a comparison. A
 * frame paints once however it was asked for. */
export function liveLoop(clock: PlaybackController, paint: () => void): { wake: () => void; stop: () => void } {
  let frame = 0, alive = true;
  const fromFrame = () => { frame = 0; if (alive) paint(); };
  // The clock's own frame paints this one: a frame asked for already is not painted twice.
  const fromClock = () => { if (frame) { cancelAnimationFrame(frame); frame = 0; } if (alive) paint(); };
  const stops = [clock.subscribeFrame(fromClock), clock.subscribe(fromClock)];
  return {
    wake: () => { if (alive && !frame && typeof requestAnimationFrame === 'function') frame = requestAnimationFrame(fromFrame); },
    stop: () => { alive = false; if (frame) cancelAnimationFrame(frame); frame = 0; for (const stop of stops) stop(); },
  };
}
