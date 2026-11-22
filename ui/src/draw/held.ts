import type { PlaybackController } from '../dashboard/playback';
import './held.css';

export function heldPicture(clock: PlaybackController, region: HTMLElement, moving: () => boolean, release: () => void, dim: () => void) {
  let shown = false, stale: 'paused' | 'bound' | null = null, holds = -Infinity;
  const mark = (next: typeof stale) => {
    if (next !== null && stale === null) dim();
    stale = next; region.dataset.held = next ? 'stale' : 'current';
    if (next) region.setAttribute('aria-busy', 'true'); else region.removeAttribute('aria-busy');
  };
  mark(null);
  const stop = clock.onAnchor((previous, next) => {
    if (!moving()) return;
    if (previous.playing && next.playing && previous.hoursPerSecond !== next.hoursPerSecond) { release(); return; }
    if (!shown) return;
    if (!next.playing && next.hour > holds) { if (stale === null) mark('paused'); }
    else if (stale === 'paused') mark(null);
  });
  return {
    get stale() { return stale !== null; },
    hold() { if (shown && stale === null) mark('bound'); },
    wait() { if (shown && stale === null && !clock.getSnapshot().playing && clock.getSnapshot().hour > holds) mark('paused'); },
    present(through: number | null | undefined) { shown = true; holds = through ?? -Infinity; mark(null); },
    drop() { shown = false; holds = -Infinity; mark(null); },
    close() { stop(); region.removeAttribute('aria-busy'); delete region.dataset.held; },
  };
}
