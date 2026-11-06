/** Images a canvas chart draws (a country's flag beside a race row), decoded once
 * per URL and shared by every chart. A chart asks for an image each frame; until it
 * has loaded, the caller chooses its placeholder and is woken when it settles. */

/** A settled image no chart has drawn for this long is dropped on the next new URL.
 * Retention follows what charts draw, never a count, so no working set thrashes. */
const IDLE_MS = 120_000;
/** Least recently drawn first. */
const images = new Map<string, { image: HTMLImageElement; drawn: number }>();
const waiting = new Map<string, Set<() => void>>();

/** The decoded image for `url`, or null while it loads (or if it cannot); `loaded`
 * is called once when it arrives. */
export function canvasImage(url: string, loaded: () => void): HTMLImageElement | null {
  return canvasImageRead(url, loaded).image;
}

/** The same shared decoder also distinguishes pending from a completed failure. */
export function canvasImageRead(url: string, loaded: () => void): { image: HTMLImageElement | null; pending: boolean; failed: boolean } {
  const now = performance.now();
  const entry = images.get(url);
  let image: HTMLImageElement;
  if (entry) {
    image = entry.image;
    entry.drawn = now;
    images.delete(url);
    images.set(url, entry);
    if (image.complete) return { image: image.naturalWidth > 0 ? image : null, pending: false, failed: image.naturalWidth === 0 };
  } else {
    if (typeof Image === 'undefined') return { image: null, pending: false, failed: false };
    dropIdle(now);
    image = new Image();
    images.set(url, { image, drawn: now });
    const settle = () => {
      const callbacks = waiting.get(url);
      waiting.delete(url);
      for (const callback of callbacks ?? []) callback();
    };
    image.onload = settle;
    image.onerror = settle;
    image.src = url;
  }
  let callbacks = waiting.get(url);
  if (!callbacks) waiting.set(url, callbacks = new Set());
  callbacks.add(loaded);
  return { image: null, pending: true, failed: false };
}

/** Drops settled images idle past `IDLE_MS`; a decode in flight is kept for its waiters. */
function dropIdle(now: number) {
  for (const [url, { image, drawn }] of images) {
    if (now - drawn < IDLE_MS) return;
    if (image.complete) images.delete(url);
  }
}
