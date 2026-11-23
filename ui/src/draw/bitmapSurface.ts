import type { PageSurface } from '../dashboard/follow/page';

export function bitmapSurface(canvas: HTMLCanvasElement): PageSurface<ImageBitmap> {
  const context = canvas.getContext('bitmaprenderer');
  if (!context) throw new Error('This page cannot show drawn pictures: it has no bitmap renderer');
  return {
    install(bitmap) { context.transferFromImageBitmap(bitmap); },
    clear() { context.transferFromImageBitmap(null); },
  };
}

const HELD = '[data-held="stale"]';
const holders = new WeakMap<Element, Set<object>>();
const SURFACE = {};

export function holdElement(element: Element | null, held: boolean, owner: object = SURFACE) {
  if (!element) return;
  let owners = holders.get(element);
  if (held) { if (!owners) holders.set(element, owners = new Set()); owners.add(owner); }
  else owners?.delete(owner);
  if (owners?.size) { element.setAttribute('data-held', 'stale'); element.setAttribute('aria-busy', 'true'); }
  else { element.removeAttribute('data-held'); element.removeAttribute('aria-busy'); }
}

export const heldAt = (element: Element | null) => !!element?.closest(HELD);

export function watchHeld(element: Element, changed: () => void): () => void {
  const holder = element.closest('article.dash-widget, [popover]') ?? element.ownerDocument.body;
  const observer = new MutationObserver(changed);
  observer.observe(holder, { subtree: true, attributes: true, attributeFilter: ['data-held'] });
  return () => observer.disconnect();
}
