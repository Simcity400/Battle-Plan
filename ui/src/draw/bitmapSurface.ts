import type { PageSurface } from '../dashboard/follow/page';

export function bitmapSurface(canvas: HTMLCanvasElement): PageSurface<ImageBitmap> {
  const context = canvas.getContext('bitmaprenderer');
  if (!context) throw new Error('This page cannot show drawn pictures: it has no bitmap renderer');
  return {
    install(bitmap) { context.transferFromImageBitmap(bitmap); },
    clear() { context.transferFromImageBitmap(null); },
  };
}

export function holdElement(element: Element | null, held: boolean) {
  if (!element) return;
  if (held) element.setAttribute('data-held', 'stale');
  else element.removeAttribute('data-held');
}
