import { drawClock, drawPost, surfaceId } from '../dashboard/pulled.ts';
import type { PlaybackController } from '../dashboard/playback';
import type { DateFaceModel } from './dateFace';

const typeOf = (element: Element) => {
  const style = getComputedStyle(element);
  return { font: `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`, color: style.color };
};

export function attachDateFace(host: HTMLElement, date: Element, hour: Element, clock: PlaybackController, offset: number): (() => void) | null {
  if (typeof HTMLCanvasElement.prototype.transferControlToOffscreen !== 'function') return null;
  const id = surfaceId(), canvas = document.createElement('canvas');
  canvas.className = 'date-time-face';
  canvas.setAttribute('aria-hidden', 'true');
  host.append(canvas);
  const offscreen = canvas.transferControlToOffscreen();
  const stopClock = drawClock(clock);
  let first = true;
  const send = () => {
    const { width, height } = host.getBoundingClientRect();
    if (!width || !height) return;
    const face: DateFaceModel = { offset, date: typeOf(date), hour: typeOf(hour) };
    drawPost({ type: 'dateFace', id, width, height, dpr: devicePixelRatio, face, ...(first && { canvas: offscreen }) }, first ? [offscreen] : []);
    first = false;
  };
  send();
  const resize = new ResizeObserver(send);
  resize.observe(host);
  const theme = new MutationObserver(send);
  theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] });
  const scheme = matchMedia('(prefers-color-scheme: dark)');
  scheme.addEventListener('change', send);
  return () => {
    resize.disconnect(); theme.disconnect(); scheme.removeEventListener('change', send); stopClock();
    drawPost({ type: 'detach', id }); canvas.remove();
  };
}
