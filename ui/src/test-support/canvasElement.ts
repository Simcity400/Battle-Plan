import type { DrawingCanvas } from '../charts/surface';

const elements = new WeakMap<OffscreenCanvas, HTMLCanvasElement>();
if (typeof HTMLCanvasElement !== 'undefined' && typeof HTMLCanvasElement.prototype.transferControlToOffscreen === 'function') {
  const original = HTMLCanvasElement.prototype.transferControlToOffscreen;
  HTMLCanvasElement.prototype.transferControlToOffscreen = function (this: HTMLCanvasElement) {
    const offscreen = original.call(this);
    elements.set(offscreen, this);
    return offscreen;
  };
}

export function canvasElement(canvas: DrawingCanvas): HTMLCanvasElement | null {
  return typeof HTMLCanvasElement !== 'undefined' && canvas instanceof HTMLCanvasElement ? canvas : elements.get(canvas as OffscreenCanvas) ?? null;
}
