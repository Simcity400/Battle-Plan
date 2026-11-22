import { clockGeneration, drawClock, drawPost, onDraw, onSurface, surfaceId } from '../dashboard/pulled.ts';
import type { PlaybackController } from '../dashboard/playback';
import type { PanelModel } from './panels/view';
import { perfLog } from '../shell/perf.ts';
import { pictureHour } from './pictureDate.ts';
import type { PictureInterval } from './pictureDate';
import { cancelGeometry, invalidateGeometry } from '../charts/geometry.ts';
import { heldPicture } from './held.ts';
import { noticeMatchesAnchor, shownTime } from './anchorGeneration.ts';

export interface PanelInspection { texts: ReadonlyMap<string, string>; live: ReadonlyMap<string, number | string>; date: PictureInterval }

export function attachPanel(content: HTMLElement, clock: PlaybackController, model: (id: number, owner: number) => PanelModel | null,
  own: (visible: boolean, pending?: boolean) => void, hit: (key: string | null) => void, hold: () => void) {
  if (typeof HTMLCanvasElement.prototype.transferControlToOffscreen !== 'function') return;
  const id = surfaceId(), canvas = document.createElement('canvas');
  content.setAttribute('data-ink-content', '');
  canvas.className = 'draw-panel-ink';
  canvas.setAttribute('aria-hidden', 'true');
  canvas.hidden = true;
  content.append(canvas);
  const offscreen = canvas.transferControlToOffscreen();
  const stopClock = drawClock(clock);
  let owner = 0, fixed: number | undefined, follows = false, closed = false, first = true, rebound = false;
  let origin = { x: 0, y: 0 };
  const visible = (show: boolean) => {
    if (canvas.hidden === !show) return;
    canvas.hidden = !show; perfLog?.ink(id, show);
  };
  const release = () => { rebound = false; held.drop(); visible(false); own(false, true); hit(null); };
  const held = heldPicture(clock, content, () => fixed === undefined, release, () => { hit(null); hold(); });
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
      fixed = reading.fixed; follows = reading.follow !== undefined;
      drawPost({ type: 'surface', id, owner, width, height, dpr: devicePixelRatio, model: { ...reading, cells },
        ...(first && { canvas: offscreen }) }, first ? [offscreen] : []);
      if (reading.widget) { canvas.dataset.placement = String(id); canvas.dataset.surface = reading.view; }
      first = false;
    };
  };
  const refresh = () => { if (!closed) { owner++; invalidateGeometry(measure); } };
  let request = 0, inspecting: { request: number; done: (picture?: PanelInspection) => void } | undefined;
  const settle = (picture?: PanelInspection) => { inspecting?.done(picture); inspecting = undefined; };
  const rebind = (exact: boolean) => {
    if (closed) return;
    settle();
    if (!exact && !canvas.hidden) { rebound = true; held.hold(); }
    else { rebound = false; held.drop(); visible(false); own(false, !exact); hit(null); }
    refresh();
  };
  const inspect = () => new Promise<PanelInspection | undefined>(done => {
    settle();
    if (closed || canvas.hidden || held.stale) { done(undefined); return; }
    inspecting = { request: ++request, done };
    drawPost({ type: 'inspectPanel', id, owner, request });
  });
  const offPage = onDraw(message => { if (message.type === 'failed' && message.page === -1 || message.type === 'followFailure' && follows) { settle(); release(); } });
  const off = onSurface(id, notice => {
    if (notice.type === 'failed') { settle(); release(); return; }
    if (notice.type === 'hit') { if (notice.owner === owner && !canvas.hidden && !held.stale) hit(notice.key); return; }
    if (notice.type === 'panelInspection') {
      if (inspecting?.request === notice.request) settle(notice.values && notice.date && !canvas.hidden && !held.stale
        ? { texts: new Map(notice.values), live: new Map(notice.live ?? []), date: notice.date } : undefined);
      return;
    }
    if (notice.type !== 'present') return;
    const p = notice.surface;
    if (p.owner !== owner || p.seek !== clock.getSnapshot().seekSequence || !noticeMatchesAnchor(p.generation, clockGeneration(clock))) return;
    const clockHour = clock.positionAt(shownTime(p.shown));
    if (fixed === undefined && p.head !== undefined && Math.floor(p.head) > Math.floor(clockHour)) { settle(); release(); return; }
    const hour = pictureHour(p.date, clockHour, fixed);
    if (hour === null && (p.waiting || rebound) && !canvas.hidden) { if (!rebound) held.wait(); return; }
    settle(); rebound = false;
    if (hour === null) held.drop(); else held.present(p.holdsThrough);
    visible(hour !== null);
    own(hour !== null, hour === null);
    if (hour === null) hit(null);
  });
  const point = (e: PointerEvent) => {
    if (canvas.hidden || held.stale || !e.isTrusted) return;
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
  refresh();
  return { refresh, rebind, inspect, get stale() { return held.stale; }, close() {
    closed = true; settle(); cancelGeometry(measure); release(); off(); offPage(); stopAnchor(); held.close(); stopClock(); resize.disconnect();
    content.removeEventListener('pointermove', point); content.removeEventListener('pointerleave', leave); content.removeEventListener('toggle', refresh, true);
    drawPost({ type: 'detach', id }); canvas.remove(); content.removeAttribute('data-ink-content');
  } };
}
