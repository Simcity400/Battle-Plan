import type { PlaybackController } from '../dashboard/playback';
import { motionOf } from '../charts/easing.ts';
import './held.css';

export function heldPicture(clock: PlaybackController, region: HTMLElement, moving: () => boolean, release: () => void, dim: () => void) {
  let shown = false, stale = false, holds = -Infinity;
  const mark = (next: boolean) => {
    stale = next; region.dataset.held = next ? 'stale' : 'current';
    if (next) region.setAttribute('aria-busy', 'true'); else region.removeAttribute('aria-busy');
  };
  mark(false);
  const stop = clock.onAnchor((previous, next) => {
    if (!moving()) return;
    const past = !next.playing && next.hour > holds;
    if (motionOf(previous) !== motionOf(next) || past && !shown) release();
    else if (past && !stale) { mark(true); dim(); }
  });
  return {
    get stale() { return stale; },
    present(through: number | null | undefined) { shown = true; holds = through ?? -Infinity; mark(false); },
    drop() { shown = false; holds = -Infinity; mark(false); },
    close() { stop(); region.removeAttribute('aria-busy'); delete region.dataset.held; },
  };
}
