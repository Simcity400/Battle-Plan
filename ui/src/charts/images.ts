/** Images a canvas chart draws (a country's flag beside a race row), decoded once
 * per URL and shared by every chart. A chart asks for an image each frame; until it
 * has loaded, the caller chooses its placeholder and is woken when it settles. */

/** Most images kept; the least recently drawn is dropped first. */
const MAX_IMAGES = 256;
const images = new Map<string, HTMLImageElement>();
const waiting = new Map<string, Set<() => void>>();

/** The decoded image for `url`, or null while it loads (or if it cannot); `loaded`
 * is called once when it arrives. */
export function canvasImage(url: string, loaded: () => void): HTMLImageElement | null {
  return canvasImageRead(url, loaded).image;
}

/** The same shared decoder also distinguishes pending from a completed failure. */
export function canvasImageRead(url: string, loaded: () => void): { image: HTMLImageElement | null; pending: boolean; failed: boolean } {
  let image = images.get(url);
  if (image) {
    // Most recently used last.
    images.delete(url);
    images.set(url, image);
    if (image.complete) return { image: image.naturalWidth > 0 ? image : null, pending: false, failed: image.naturalWidth === 0 };
  } else {
    if (typeof Image === 'undefined') return { image: null, pending: false, failed: false };
    image = new Image();
    images.set(url, image);
    const settle = () => {
      const callbacks = waiting.get(url);
      waiting.delete(url);
      for (const callback of callbacks ?? []) callback();
    };
    image.onload = settle;
    image.onerror = settle;
    image.src = url;
    while (images.size > MAX_IMAGES) images.delete(images.keys().next().value!);
  }
  let callbacks = waiting.get(url);
  if (!callbacks) waiting.set(url, callbacks = new Set());
  callbacks.add(loaded);
  return { image: null, pending: true, failed: false };
}
