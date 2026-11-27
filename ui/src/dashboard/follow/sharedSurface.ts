import { createContext } from 'react';
import type { SharedSurfaceSlot } from '../../draw/laneMessages.ts';
import type { Pipeline } from './pipeline.ts';

export interface GridRectItem {
  readonly id: string;
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface SpatialLanePartition {
  readonly laneById: ReadonlyMap<string, number>;
  readonly hasCleanCut: boolean;
}

function findBestCut(
  items: readonly GridRectItem[],
  axis: 'x' | 'y',
): { coord: number; left: GridRectItem[]; right: GridRectItem[]; imb: number } | null {
  const coords = new Set<number>();
  for (const item of items) {
    coords.add(axis === 'x' ? item.x : item.y);
    coords.add(axis === 'x' ? item.x + item.w : item.y + item.h);
  }
  const sorted = [...coords].sort((a, b) => a - b);
  let best: { coord: number; left: GridRectItem[]; right: GridRectItem[]; imb: number } | null = null;
  for (let i = 1; i < sorted.length - 1; i++) {
    const c = sorted[i]!;
    let clean = true;
    const left: GridRectItem[] = [];
    const right: GridRectItem[] = [];
    for (const item of items) {
      const start = axis === 'x' ? item.x : item.y;
      const end = axis === 'x' ? item.x + item.w : item.y + item.h;
      if (start < c && end > c) {
        clean = false;
        break;
      }
      if (end <= c) left.push(item);
      else right.push(item);
    }
    if (!clean || !left.length || !right.length) continue;
    const imb = Math.abs(left.length - right.length);
    if (!best || imb < best.imb) best = { coord: c, left, right, imb };
  }
  return best;
}

function partitionInto(
  items: readonly GridRectItem[],
  lanes: readonly number[],
  out: Map<string, number>,
): boolean {
  if (!items.length || !lanes.length) return true;
  if (lanes.length === 1 || items.length === 1) {
    const lane = lanes[0]!;
    for (const item of items) out.set(item.id, lane);
    return true;
  }
  const cutY = findBestCut(items, 'y');
  const cutX = findBestCut(items, 'x');
  const best = cutY && cutX ? (cutY.imb <= cutX.imb ? cutY : cutX) : (cutY ?? cutX);
  if (best) {
    const leftCount = Math.max(1, Math.min(lanes.length - 1, Math.round((lanes.length * best.left.length) / items.length)));
    const okLeft = partitionInto(best.left, lanes.slice(0, leftCount), out);
    const okRight = partitionInto(best.right, lanes.slice(leftCount), out);
    return okLeft && okRight;
  }
  const sorted = [...items].sort((a, b) => a.y - b.y || a.x - b.x);
  const per = Math.ceil(sorted.length / lanes.length);
  for (let i = 0; i < sorted.length; i++) {
    const laneIdx = Math.min(lanes.length - 1, Math.floor(i / per));
    out.set(sorted[i]!.id, lanes[laneIdx]!);
  }
  return false;
}

export function computeSpatialLaneMap(items: readonly GridRectItem[], laneCount: number): SpatialLanePartition {
  const laneById = new Map<string, number>();
  const effective = Math.max(1, Math.min(laneCount, Math.max(1, items.length)));
  const lanes = Array.from({ length: effective }, (_, i) => i);
  const hasCleanCut = partitionInto(items, lanes, laneById);
  return { laneById, hasCleanCut };
}

export function isImageBackdropActive(): boolean {
  try {
    return typeof document !== 'undefined' && document.documentElement.dataset.backdrop === 'image';
  } catch {
    return false;
  }
}

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface SharedSlotInput {
  readonly widget: number;
  readonly lane: number;
  readonly rect: Rect;
  readonly inView: boolean;
  readonly active: boolean;
}

export interface SharedLaneLayout {
  readonly lane: number;
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
  readonly clipPath: string;
  readonly clipPathFallback: boolean;
  readonly slots: readonly SharedSurfaceSlot[];
}

export interface SharedGeometryResult {
  readonly clipPathFallback: boolean;
  readonly lanes: ReadonlyMap<number, SharedLaneLayout>;
}

function rectsOverlap(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w - 0.5 && a.x + a.w > b.x + 0.5 && a.y < b.y + b.h - 0.5 && a.y + a.h > b.y + 0.5;
}

function snapFloor(v: number, dpr: number): number {
  return Math.floor(v * dpr) / dpr;
}

function snapCeil(v: number, dpr: number): number {
  return Math.ceil(v * dpr) / dpr;
}

function snapRound(v: number, dpr: number): number {
  return Math.round(v * dpr) / dpr;
}

export function computeSharedLaneGeometry(
  items: readonly SharedSlotInput[],
  viewport: Rect,
  dpr: number,
): SharedGeometryResult {
  const perLane = new Map<number, SharedSlotInput[]>();
  for (const item of items) {
    let list = perLane.get(item.lane);
    if (!list) perLane.set(item.lane, list = []);
    list.push(item);
  }

  const bounds = new Map<number, Rect>();
  const minXLimit = snapFloor(viewport.x, dpr);
  const minYLimit = snapFloor(viewport.y, dpr);
  const maxXLimit = snapCeil(viewport.x + viewport.w, dpr);
  const maxYLimit = snapCeil(viewport.y + viewport.h, dpr);

  for (const [lane, list] of perLane) {
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const item of list) {
      if (!item.inView || !item.active) continue;
      const cx0 = Math.max(minXLimit, item.rect.x);
      const cy0 = Math.max(minYLimit, item.rect.y);
      const cx1 = Math.min(maxXLimit, item.rect.x + item.rect.w);
      const cy1 = Math.min(maxYLimit, item.rect.y + item.rect.h);
      if (cx1 <= cx0 || cy1 <= cy0) continue;
      if (cx0 < minX) minX = cx0;
      if (cy0 < minY) minY = cy0;
      if (cx1 > maxX) maxX = cx1;
      if (cy1 > maxY) maxY = cy1;
    }
    if (minX < maxX && minY < maxY) {
      const bx = snapFloor(minX, dpr);
      const by = snapFloor(minY, dpr);
      const bw = snapCeil(maxX, dpr) - bx;
      const bh = snapCeil(maxY, dpr) - by;
      bounds.set(lane, { x: bx, y: by, w: Math.max(1, bw), h: Math.max(1, bh) });
    }
  }

  let overlap = false;
  const boundList = [...bounds.values()];
  for (let i = 0; i < boundList.length && !overlap; i++) {
    for (let j = i + 1; j < boundList.length; j++) {
      if (rectsOverlap(boundList[i]!, boundList[j]!)) {
        overlap = true;
        break;
      }
    }
  }

  const lanes = new Map<number, SharedLaneLayout>();
  const vpW = Math.max(0, Math.round(viewport.w));
  const vpH = Math.max(0, Math.round(viewport.h));

  for (const [lane, b] of bounds) {
    const list = perLane.get(lane) ?? [];
    const ox = overlap ? minXLimit : b.x;
    const oy = overlap ? minYLimit : b.y;
    const cw = overlap ? vpW : b.w;
    const ch = overlap ? vpH : b.h;
    const slots: SharedSurfaceSlot[] = [];
    const clipParts: string[] = [];
    let singleSlot: SharedSurfaceSlot | null = null;
    for (const item of list) {
      const sx = snapRound(item.rect.x - ox, dpr);
      const sy = snapRound(item.rect.y - oy, dpr);
      const sw = item.rect.w;
      const sh = item.rect.h;
      const slot: SharedSurfaceSlot = { widget: item.widget, x: sx, y: sy, w: sw, h: sh, visible: item.active };
      slots.push(slot);
      if (item.active) {
        clipParts.push(`M${sx} ${sy}H${sx + sw}V${sy + sh}H${sx}Z`);
        singleSlot = slot;
      }
    }
    const needsClip = overlap
      || clipParts.length > 1
      || (clipParts.length === 1 && singleSlot !== null && (
        singleSlot.x > 0.5 || singleSlot.y > 0.5 || singleSlot.w < cw - 0.5 || singleSlot.h < ch - 0.5
      ));
    lanes.set(lane, {
      lane,
      left: ox,
      top: oy,
      width: cw,
      height: ch,
      clipPath: needsClip && clipParts.length ? `path("${clipParts.join(' ')}")` : '',
      clipPathFallback: overlap,
      slots,
    });
  }

  return { clipPathFallback: overlap, lanes };
}

interface RegisteredSlot {
  readonly widget: number;
  readonly lane: number;
  readonly bodyEl: HTMLElement;
  readonly frameEl: HTMLElement;
  active: boolean;
}

interface LaneCanvasState {
  readonly lane: number;
  readonly canvas: HTMLCanvasElement;
  transferred: boolean;
  lastSig: string;
}

function readSurfaceColor(root: HTMLElement): string {
  try {
    const cs = getComputedStyle(root);
    const bg = cs.getPropertyValue('--bg').trim() || cs.getPropertyValue('--canvas').trim() || cs.backgroundColor;
    return bg && bg !== 'transparent' && !/^rgba\(\s*0\s*,\s*0\s*,\s*0\s*,\s*0\s*\)$/.test(bg) ? bg : '';
  } catch {
    return '';
  }
}

function supportsOffscreenTransfer(): boolean {
  try {
    return typeof HTMLCanvasElement !== 'undefined'
      && typeof HTMLCanvasElement.prototype.transferControlToOffscreen === 'function';
  } catch {
    return false;
  }
}

export class SharedSurfaceHost {
  private readonly pipeline: Pipeline;
  private readonly scrollEl: HTMLElement;
  private readonly layerEl: HTMLDivElement;
  private readonly lanes = new Map<number, LaneCanvasState>();
  private readonly widgets = new Map<number, RegisteredSlot>();
  private readonly ro: ResizeObserver | null;
  private readonly mo: MutationObserver | null;
  private readonly supported: boolean;
  private rafId: number | null = null;
  private disposed = false;
  private imageBackdrop = isImageBackdropActive();
  private interactive = false;
  private lastUsedClipPath = false;
  private readonly failedLanes = new Set<number>();
  private readonly listeners = new Set<() => void>();
  private readonly unsubLaneFailed: (() => void) | null;

  constructor(pipeline: Pipeline, scrollEl: HTMLElement) {
    this.pipeline = pipeline;
    this.scrollEl = scrollEl;
    this.supported = supportsOffscreenTransfer();
    this.unsubLaneFailed = pipeline.onLaneSurfaceFailed?.(lane => this.failLane(lane)) ?? null;
    const layer = document.createElement('div');
    layer.className = 'dash-shared-layer';
    layer.setAttribute('aria-hidden', 'true');
    scrollEl.insertBefore(layer, scrollEl.firstChild);
    this.layerEl = layer;

    scrollEl.addEventListener('scroll', this.onScroll, { passive: true });
    if (typeof ResizeObserver !== 'undefined') {
      this.ro = new ResizeObserver(() => this.scheduleSync());
      this.ro.observe(scrollEl);
    } else {
      this.ro = null;
    }
    if (typeof MutationObserver !== 'undefined' && typeof document !== 'undefined') {
      this.mo = new MutationObserver(() => {
        const next = isImageBackdropActive();
        if (next !== this.imageBackdrop) {
          this.imageBackdrop = next;
          for (const listener of this.listeners) listener();
          this.scheduleSync();
        }
      });
      this.mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-backdrop', 'data-theme', 'data-mode'] });
    } else {
      this.mo = null;
    }
  }

  private failLane(lane: number): void {
    if (this.disposed || this.failedLanes.has(lane)) return;
    this.failedLanes.add(lane);
    const existing = this.lanes.get(lane);
    if (existing) {
      existing.canvas.remove();
      this.lanes.delete(lane);
    }
    for (const listener of this.listeners) listener();
    this.scheduleSync();
  }

  private ensureLane(lane: number): LaneCanvasState | null {
    const existing = this.lanes.get(lane);
    if (existing) return existing;
    if (!this.supported || this.failedLanes.has(lane) || lane < 0 || lane >= this.pipeline.laneCount) return null;
    const canvas = document.createElement('canvas');
    canvas.className = 'dash-shared-canvas';
    canvas.dataset.sharedLane = String(lane);
    canvas.width = 1;
    canvas.height = 1;
    this.layerEl.appendChild(canvas);
    let transferred = false;
    try {
      const offscreen = canvas.transferControlToOffscreen();
      this.pipeline.transferLaneSurface(lane, offscreen, true);
      transferred = true;
    } catch {
      transferred = false;
    }
    if (!transferred) {
      canvas.remove();
      this.failedLanes.add(lane);
      for (const listener of this.listeners) listener();
      return null;
    }
    const state: LaneCanvasState = { lane, canvas, transferred, lastSig: '' };
    this.lanes.set(lane, state);
    return state;
  }

  get enabled(): boolean {
    return !this.disposed && !this.imageBackdrop && !this.interactive && this.supported;
  }

  get laneCount(): number {
    return this.pipeline.laneCount;
  }

  laneAvailable(lane: number | null): boolean {
    if (!this.enabled) return false;
    return lane === null || !this.failedLanes.has(lane);
  }

  setInteractive(active: boolean): void {
    if (this.disposed || this.interactive === active) return;
    this.interactive = active;
    for (const listener of this.listeners) listener();
    this.scheduleSync();
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  get usingClipPathFallback(): boolean {
    return this.lastUsedClipPath;
  }

  register(widget: number, lane: number, bodyEl: HTMLElement, frameEl: HTMLElement): () => void {
    this.ensureLane(lane);
    const entry: RegisteredSlot = { widget, lane, bodyEl, frameEl, active: true };
    this.widgets.set(widget, entry);
    this.scheduleSync();
    return () => {
      if (this.widgets.get(widget) === entry) {
        this.widgets.delete(widget);
        delete frameEl.dataset.sharedSurface;
        delete frameEl.dataset.sharedLane;
        this.scheduleSync();
      }
    };
  }

  setActive(widget: number, active: boolean): void {
    const entry = this.widgets.get(widget);
    if (!entry || entry.active === active) return;
    entry.active = active;
    if (!active) {
      delete entry.frameEl.dataset.sharedSurface;
      delete entry.frameEl.dataset.sharedLane;
    }
    this.scheduleSync();
  }

  private readonly onScroll = () => {
    this.scheduleSync();
  };

  scheduleSync(): void {
    if (this.disposed || this.rafId !== null) return;
    let done = false;
    const id = requestAnimationFrame(() => {
      done = true;
      this.rafId = null;
      this.syncNow();
    });
    if (!done) this.rafId = id;
  }

  syncNow(): void {
    if (this.disposed) return;
    const dpr = window.devicePixelRatio || 1;
    const scrollRect = this.scrollEl.getBoundingClientRect();
    const layerRect = this.layerEl.getBoundingClientRect();
    const vpW = Math.max(0, Math.round(scrollRect.width));
    const vpH = Math.max(0, Math.round(scrollRect.height));
    const scrollTop = this.scrollEl.scrollTop;
    const scrollLeft = this.scrollEl.scrollLeft;
    const surfaceColor = readSurfaceColor(this.scrollEl);
    const activeEnabled = this.enabled && vpW > 0 && vpH > 0;

    const inputs: SharedSlotInput[] = [];
    for (const entry of this.widgets.values()) {
      const dragging = Boolean(entry.frameEl.closest('.react-draggable-dragging, .resizing'));
      if (!activeEnabled || dragging) {
        delete entry.frameEl.dataset.sharedSurface;
        delete entry.frameEl.dataset.sharedLane;
        continue;
      }
      const laneState = this.ensureLane(entry.lane);
      if (!laneState?.transferred) {
        delete entry.frameEl.dataset.sharedSurface;
        delete entry.frameEl.dataset.sharedLane;
        continue;
      }
      const r = entry.bodyEl.getBoundingClientRect();
      const ow = entry.bodyEl.offsetWidth;
      const oh = entry.bodyEl.offsetHeight;
      const cw = entry.bodyEl.clientWidth;
      const ch = entry.bodyEl.clientHeight;
      const zoomX = ow > 0 && cw > 0 ? r.width / ow : 1;
      const zoomY = oh > 0 && ch > 0 ? r.height / oh : 1;
      const contentLeft = r.left + (entry.bodyEl.clientLeft || 0) * zoomX;
      const contentTop = r.top + (entry.bodyEl.clientTop || 0) * zoomY;
      const contentW = ow > 0 && cw > 0 ? cw * zoomX : r.width;
      const contentH = oh > 0 && ch > 0 ? ch * zoomY : r.height;
      const x = snapRound(contentLeft - layerRect.left, dpr);
      const y = snapRound(contentTop - layerRect.top, dpr);
      const w = snapRound(contentW, dpr);
      const h = snapRound(contentH, dpr);
      const inView = w > 0 && h > 0 && contentLeft + contentW > scrollRect.left && contentLeft < scrollRect.right && contentTop + contentH > scrollRect.top && contentTop < scrollRect.bottom;
      const visible = inView && entry.active;
      if (visible) {
        entry.frameEl.dataset.sharedSurface = '1';
        entry.frameEl.dataset.sharedLane = String(entry.lane);
      } else {
        delete entry.frameEl.dataset.sharedSurface;
        delete entry.frameEl.dataset.sharedLane;
      }
      inputs.push({ widget: entry.widget, lane: entry.lane, rect: { x, y, w, h }, inView, active: visible });
    }

    const geometry = computeSharedLaneGeometry(
      inputs,
      { x: scrollRect.left - layerRect.left, y: scrollRect.top - layerRect.top, w: scrollRect.width, h: scrollRect.height },
      dpr,
    );
    this.lastUsedClipPath = geometry.clipPathFallback;

    for (const laneState of this.lanes.values()) {
      if (!laneState.transferred) continue;
      const layout = activeEnabled ? geometry.lanes.get(laneState.lane) : undefined;
      if (!layout) {
        laneState.canvas.style.display = 'none';
        laneState.canvas.style.clipPath = '';
        const sig = 'off';
        if (laneState.lastSig !== sig) {
          laneState.lastSig = sig;
          this.pipeline.sendLaneSlots(laneState.lane, {
            width: 0,
            height: 0,
            dpr,
            scrollTop,
            scrollLeft,
            surfaceColor,
            clipPathFallback: false,
            slots: [],
          });
        }
        continue;
      }

      const originDx = (Math.round((layerRect.left + layout.left) * dpr) - (layerRect.left + layout.left) * dpr) / dpr;
      const originDy = (Math.round((layerRect.top + layout.top) * dpr) - (layerRect.top + layout.top) * dpr) / dpr;

      laneState.canvas.style.display = 'block';
      laneState.canvas.style.left = `${layout.left}px`;
      laneState.canvas.style.top = `${layout.top}px`;
      laneState.canvas.style.width = `${layout.width}px`;
      laneState.canvas.style.height = `${layout.height}px`;
      laneState.canvas.style.transform = Math.abs(originDx) > 1e-4 || Math.abs(originDy) > 1e-4
        ? `translate(${originDx}px, ${originDy}px)`
        : '';
      laneState.canvas.style.clipPath = layout.clipPath;
      laneState.canvas.dataset.clipFallback = layout.clipPathFallback ? '1' : '0';

      const originX = snapRound(scrollLeft + layout.left, dpr);
      const originY = snapRound(scrollTop + layout.top, dpr);
      const sig = `${layout.width}:${layout.height}:${dpr}:${originX}:${originY}:${surfaceColor}:${layout.clipPathFallback ? 1 : 0}|${layout.slots.map(s => `${s.widget},${s.x},${s.y},${s.w},${s.h},${s.visible ? 1 : 0}`).join(';')}`;
      if (laneState.lastSig !== sig) {
        laneState.lastSig = sig;
        this.pipeline.sendLaneSlots(laneState.lane, {
          width: layout.width,
          height: layout.height,
          dpr,
          scrollTop,
          scrollLeft,
          originX,
          originY,
          surfaceColor,
          clipPathFallback: layout.clipPathFallback,
          slots: layout.slots,
        });
      }
    }
  }

  dispose(): void {
    this.disposed = true;
    if (this.rafId !== null) cancelAnimationFrame(this.rafId);
    this.scrollEl.removeEventListener('scroll', this.onScroll);
    this.ro?.disconnect();
    this.mo?.disconnect();
    this.unsubLaneFailed?.();
    for (const laneState of this.lanes.values()) {
      if (laneState.transferred) this.pipeline.disposeLaneSurface?.(laneState.lane);
    }
    for (const entry of this.widgets.values()) {
      delete entry.frameEl.dataset.sharedSurface;
      delete entry.frameEl.dataset.sharedLane;
    }
    this.widgets.clear();
    this.lanes.clear();
    this.layerEl.remove();
  }
}

export const SharedSurfaceContext = createContext<SharedSurfaceHost | null>(null);
export const SpatialLaneContext = createContext<ReadonlyMap<string, number> | null>(null);
