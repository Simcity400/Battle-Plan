import { clockGeneration, drawClock, drawPost, onDraw, surfaceId } from '../../dashboard/pulled.ts';
import { noticeMatchesAnchor } from '../anchorGeneration.ts';
import { perfLog } from '../../shell/perf.ts';

export type ExactRows = (hour: number) => Promise<readonly string[] | null>;
import type { PlaybackController } from '../../dashboard/playback';
import type { TableReading, TableSurfaceModel } from './surface.ts';
import { ROW_HEIGHT } from './logic.ts';

export interface TableHostHandle {
  refresh(): void;
  read(ask: { keys?: readonly string[]; rows?: { first: number; count: number }; detail?: string }): Promise<TableReading | null>;
  close(): void;
}

export function attachTable(body: HTMLElement, clock: PlaybackController,
  model: (id: number, owner: number, width: number, height: number) => TableSurfaceModel | null,
  own: (visible: boolean, pending?: boolean) => void, hover: (key: string | null, at?: { x: number; y: number }) => void,
  scrolled: (left: number) => void, extentWidth: () => number, exactRows?: () => ExactRows | undefined,
  drawnRows?: (count: number, empty?: string) => void, drawnWidths?: (widths: Record<string, number>) => void): TableHostHandle | null {
  if (typeof HTMLCanvasElement.prototype.transferControlToOffscreen !== 'function') return null;
  const id = surfaceId(), extent = document.createElement('div'), canvas = document.createElement('canvas'), offscreen = canvas.transferControlToOffscreen();
  extent.className = 'tf-extent';
  canvas.className = 'tf-canvas'; canvas.setAttribute('aria-hidden', 'true'); extent.append(canvas); body.append(extent);
  canvas.dataset.surface = 'table'; canvas.dataset.placement = String(id);
  let owner = 0, closed = false, first = true, shown = false, at: { x: number; y: number } | null = null, request = 0;
  const reads = new Map<number, (reading: TableReading | null) => void>();
  const size = () => ({ width: Math.max(1, body.clientWidth), height: Math.max(1, body.clientHeight) });
  let rowSize = { height: 0, row: 0, most: Infinity }, rowCount = 0, extentHeight = -1;
  const extend = (count: number, fit = !clock.getSnapshot().playing) => {
    rowCount = count;
    const next = Math.max(rowSize.height, rowSize.row * Math.min(count, rowSize.most));
    if (next === extentHeight || next < extentHeight && !fit) return;
    extentHeight = next; extent.style.height = `${next}px`;
  };
  const refresh = () => {
    if (closed) return;
    owner++;
    const { width, height } = size(), made = model(id, owner, width, height);
    if (!made) { own(false, true); return; }
    const exact = exactRows?.();
    if (exact) perfLog?.reader(id, owner, async hour => { const keys = await exact(hour); return keys && { values: new Map(keys.map(key => [key, key])) }; });
    made.scrollLeft = body.scrollLeft; made.scrollTop = body.scrollTop; made.headerHeight = 0;
    rowSize = { height, row: ROW_HEIGHT[made.settings.density], most: made.members?.most ?? Infinity };
    extent.style.width = `${Math.max(width, extentWidth())}px`;
    extend(made.rows.length, true);
    Object.assign(canvas.style, { width: `${width}px`, height: `${height}px` });
    drawPost({ type: 'table', id, owner, width, height, dpr: devicePixelRatio, model: made, ...(first && { canvas: offscreen }) }, first ? [offscreen] : []);
    first = false;
  };
  const stopClock = drawClock(clock);
  let asked = { seek: clock.getSnapshot().seekSequence, playing: clock.getSnapshot().playing };
  const stopFit = clock.subscribe(() => {
    const now = clock.getSnapshot();
    if (!now.playing) extend(rowCount, true);
    if (now.seekSequence === asked.seek && now.playing === asked.playing) return;
    asked = { seek: now.seekSequence, playing: now.playing };
    if (shown) own(true, true);
  });
  const off = onDraw(message => {
    if (message.type === 'failed' && (message.page === -1 || message.page === id)) { own(false, true); return; }
    if (message.type === 'hit' && message.id === id && message.owner === owner && shown) hover(at ? message.key : null, at ?? undefined);
    if (message.type === 'tableRows' && message.id === id && message.owner === owner) { extend(message.count); drawnRows?.(message.count, message.empty); }
    if (message.type === 'tableColumns' && message.id === id && message.owner === owner) drawnWidths?.(message.widths);
    if (message.type === 'tableRead' && message.id === id) {
      reads.get(message.request)?.(message.owner === owner ? message.reading : null);
      reads.delete(message.request);
    }
    if (message.type !== 'present') return;
    for (const surface of message.surfaces) {
      if (surface.id !== id || surface.owner !== owner || surface.seek !== clock.getSnapshot().seekSequence
        || !noticeMatchesAnchor(surface.generation, clockGeneration(clock))) continue;
      const now = surface.readHour !== undefined;
      if (now !== shown) perfLog?.ink(id, now);
      shown = now;
      own(shown, !shown);
      if (!shown) hover(null);
    }
  });
  const point = () => {
    if (!at) return;
    const box = body.getBoundingClientRect();
    drawPost({ type: 'tablePointer', id, owner, scrollLeft: body.scrollLeft, point: [at.x - box.left, at.y - box.top] });
  };
  const pointer = (event: PointerEvent) => {
    if (!shown || !event.isTrusted) return;
    at = { x: event.clientX, y: event.clientY }; point();
  };
  const leave = () => { at = null; hover(null); drawPost({ type: 'tablePointer', id, owner, scrollLeft: body.scrollLeft, point: null }); };
  const onScroll = () => { scrolled(body.scrollLeft); drawPost({ type: 'tableScroll', id, owner, scrollLeft: body.scrollLeft, scrollTop: body.scrollTop }); point(); };
  body.addEventListener('pointermove', pointer); body.addEventListener('pointerleave', leave); body.addEventListener('scroll', onScroll, { passive: true });
  const observer = new ResizeObserver(refresh); observer.observe(body); refresh();
  const read: TableHostHandle['read'] = ask => closed ? Promise.resolve(null) : new Promise(resolve => {
    reads.set(++request, resolve);
    drawPost({ type: 'tableRead', id, owner, request, ...ask });
  });
  return { refresh, read, close() {
    if (closed) return;
    closed = true; own(false); hover(null); off(); stopClock(); stopFit(); observer.disconnect();
    for (const resolve of reads.values()) resolve(null);
    reads.clear();
    body.removeEventListener('pointermove', pointer); body.removeEventListener('pointerleave', leave); body.removeEventListener('scroll', onScroll);
    drawPost({ type: 'detach', id }); extent.remove();
  } };
}
