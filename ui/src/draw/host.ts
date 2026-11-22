import { clockGeneration, drawClock, drawPost, onDraw, onSurface, surfaceId } from '../dashboard/pulled.ts';
import type { PlaybackController } from '../dashboard/playback';
import type { PanelModel } from './panels/view';
import { perfLog } from '../shell/perf.ts';
import { pictureHour } from './pictureDate.ts';
import { cancelGeometry, invalidateGeometry } from '../charts/geometry.ts';
import { noticeMatchesAnchor, shownTime } from './anchorGeneration.ts';

export function attachPanel(content: HTMLElement, clock: PlaybackController, model: (id: number, owner: number) => PanelModel | null,
  own: (visible: boolean, pending?: boolean) => void, hit: (key: string | null) => void) {
  if (typeof HTMLCanvasElement.prototype.transferControlToOffscreen !== 'function') return;
  const id = surfaceId(), canvas = document.createElement('canvas');
  content.setAttribute('data-ink-content', '');
  canvas.className = 'draw-panel-ink';
  canvas.setAttribute('aria-hidden', 'true');
  canvas.hidden = true;
  content.append(canvas);
  const offscreen = canvas.transferControlToOffscreen();
  const stopClock = drawClock(clock);
  let owner = 0, fixed: number | undefined, closed = false, first = true;
  let origin = { x: 0, y: 0 };
  const visible = (show: boolean) => {
    if (canvas.hidden === !show) return;
    canvas.hidden = !show; perfLog?.ink(id, show);
  };
  const release = () => { visible(false); own(false, true); hit(null); };
  const measure = () => {
    const reading = model(id, owner);
    if (!reading?.cells.length) return release;
    const left = Math.floor(Math.min(...reading.cells.map(c => c.x)) - 4);
    const top = Math.floor(Math.min(...reading.cells.map(c => c.y)) - 4);
    const width = Math.ceil(Math.max(...reading.cells.map(c => c.x + c.width)) + 4) - left;
    const height = Math.ceil(Math.max(...reading.cells.map(c => c.y + c.height)) + 4) - top;
    const cells = reading.cells.map(c => ({ ...c, x: c.x - left, y: c.y - top, baseline: c.baseline - top }));
    return () => {
      if (closed) return;
      origin = { x: left, y: top };
      Object.assign(canvas.style, { left: left + 'px', top: top + 'px', width: width + 'px', height: height + 'px' });
      fixed = reading.fixed;
      drawPost({ type: 'surface', id, owner, width, height, dpr: devicePixelRatio, model: { ...reading, cells },
        ...(first && { canvas: offscreen }) }, first ? [offscreen] : []);
      if (reading.widget) { canvas.dataset.placement = String(id); canvas.dataset.surface = reading.view; }
      first = false;
    };
  };
  const refresh = () => { if (!closed) { owner++; invalidateGeometry(measure); } };
  const offPage = onDraw(message => { if (message.type === 'failed' && message.page === -1) release(); });
  const off = onSurface(id, notice => {
    if (notice.type === 'failed') { release(); return; }
    if (notice.type === 'hit') { if (notice.owner === owner && !canvas.hidden) hit(notice.key); return; }
    if (notice.type !== 'present') return;
    const p = notice.surface;
    if (p.owner !== owner || p.seek !== clock.getSnapshot().seekSequence || !noticeMatchesAnchor(p.generation, clockGeneration(clock))) return;
    const clockHour = clock.positionAt(shownTime(p.shown));
    if (fixed === undefined && p.head !== undefined && Math.floor(p.head) > Math.floor(clockHour)) { release(); return; }
    const hour = pictureHour(p.date, clockHour, fixed);
    visible(hour !== null);
    own(hour !== null, hour === null);
    if (hour === null) hit(null);
  });
  const point = (e: PointerEvent) => {
    if (canvas.hidden || !e.isTrusted) return;
    const box = content.getBoundingClientRect();
    drawPost({ type: 'pointer', id, owner, point: [e.clientX - box.left - content.clientLeft + content.scrollLeft - origin.x,
      e.clientY - box.top - content.clientTop + content.scrollTop - origin.y] });
  };
  const leave = () => drawPost({ type: 'pointer', id, owner, point: null });
  content.addEventListener('pointermove', point); content.addEventListener('pointerleave', leave);
  content.addEventListener('toggle', refresh, true);
  const resize = new ResizeObserver(refresh); resize.observe(content);
  const stopAnchor = clock.onDiscard(() => {
    if (fixed === undefined) { release(); refresh(); }
  });
  const stopClockAnchor = clock.onAnchor((previous, next) => {
    if (fixed === undefined && (previous.playing !== next.playing || previous.hoursPerSecond !== next.hoursPerSecond)) release();
  });
  refresh();
  return { refresh, close() {
    closed = true; cancelGeometry(measure); release(); off(); offPage(); stopAnchor(); stopClockAnchor(); stopClock(); resize.disconnect();
    content.removeEventListener('pointermove', point); content.removeEventListener('pointerleave', leave); content.removeEventListener('toggle', refresh, true);
    drawPost({ type: 'detach', id }); canvas.remove(); content.removeAttribute('data-ink-content');
  } };
}
