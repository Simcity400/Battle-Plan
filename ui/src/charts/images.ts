
const IDLE_MS = 120_000;
const images = new Map<string, { image: HTMLImageElement; drawn: number }>();
const waiting = new Map<string, Set<() => void>>();

export function canvasImage(url: string, loaded: () => void): HTMLImageElement | null {
  return canvasImageRead(url, loaded).image;
}

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

function dropIdle(now: number) {
  for (const [url, { image, drawn }] of images) {
    if (now - drawn < IDLE_MS) return;
    if (image.complete) images.delete(url);
  }
}
