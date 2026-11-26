import type { MapBounds } from './api';
import { engine } from '../engine/frame.ts';

export interface Rect { left: number; top: number; right: number; bottom: number }

export function hasMapOverlay(): boolean {
  return !!document.querySelector('[popover]:popover-open:not([role="tooltip"]), dialog[open]');
}

export function intersectRects(...rects: Rect[]): Rect {
  return { left: Math.max(...rects.map((rect) => rect.left)), top: Math.max(...rects.map((rect) => rect.top)),
    right: Math.min(...rects.map((rect) => rect.right)), bottom: Math.min(...rects.map((rect) => rect.bottom)) };
}

export function physicalRect(rect: Rect, dpr: number): MapBounds {
  const x = Math.round(rect.left * dpr);
  const y = Math.round(rect.top * dpr);
  return { x, y, width: Math.max(0, Math.round(rect.right * dpr) - x), height: Math.max(0, Math.round(rect.bottom * dpr) - y) };
}

function subtract(area: MapBounds, hole: MapBounds): MapBounds[] {
  const right = area.x + area.width, bottom = area.y + area.height;
  const cut = { left: Math.max(area.x, hole.x), top: Math.max(area.y, hole.y),
    right: Math.min(right, hole.x + hole.width), bottom: Math.min(bottom, hole.y + hole.height) };
  if (cut.right <= cut.left || cut.bottom <= cut.top) return [area];
  return [
    { x: area.x, y: area.y, width: area.width, height: cut.top - area.y },
    { x: area.x, y: cut.bottom, width: area.width, height: bottom - cut.bottom },
    { x: area.x, y: cut.top, width: cut.left - area.x, height: cut.bottom - cut.top },
    { x: cut.right, y: cut.top, width: right - cut.right, height: cut.bottom - cut.top },
  ].filter(part => part.width > 0 && part.height > 0);
}

export function coveredBy(area: MapBounds, holes: readonly MapBounds[]): boolean {
  let left = area.width > 0 && area.height > 0 ? [area] : [];
  for (const hole of holes) left = left.flatMap(part => subtract(part, hole));
  return !left.length;
}

const PENDING = 'data-map-hole-pending';
const SURFACE = 'data-map-surface';
const OWN_CLOCK = 'own';
const CLEAR = /^(transparent|rgba\(0, 0, 0, 0\))$/;

function dims(element: HTMLElement): boolean {
  const backdrop = getComputedStyle(element, '::backdrop').backgroundColor;
  return !!backdrop && !CLEAR.test(backdrop);
}

const shown = { holes: [] as readonly MapBounds[], covered: false, windows: [] as readonly HTMLElement[], dimming: [] as readonly HTMLElement[] };
const listeners = new Set<() => void>();
let active: MapHoles | null = null;

interface Framed { element: HTMLElement; order: number; surfaces: readonly HTMLElement[] }

class MapHoles {
  private tracked = new Set<HTMLElement>();
  private waiting = new Map<HTMLElement, number>();
  private opened = new Map<HTMLElement, number>();
  private opens = 0;
  private framed: Framed[] = [];
  private watched = new Set<HTMLElement>();
  private frame: (() => void) | null = null;
  private measured = 0;
  private sent = '';
  private queued: { holes: MapBounds[]; key: string; measured: number } | null = null;
  private sending = false;
  private sizes = new ResizeObserver(() => this.schedule());
  private moves = new MutationObserver(() => this.schedule());

  private send: (holes: MapBounds[]) => Promise<void>;

  constructor(send: (holes: MapBounds[]) => Promise<void>) { this.send = send; }

  start(): () => void {
    active = this;
    document.addEventListener('beforetoggle', this.toggling, true);
    document.addEventListener('toggle', this.schedule, true);
    window.addEventListener('resize', this.moved);
    window.addEventListener('scroll', this.moved, true);
    this.schedule();
    return () => {
      document.removeEventListener('beforetoggle', this.toggling, true);
      document.removeEventListener('toggle', this.schedule, true);
      window.removeEventListener('resize', this.moved);
      window.removeEventListener('scroll', this.moved, true);
      this.frame?.();
      this.frame = null;
      for (const element of [...this.tracked]) this.drop(element);
      this.sizes.disconnect();
      this.moves.disconnect();
      this.watched.clear();
      this.framed = [];
      if (active === this) active = null;
      this.publish([], false, [], []);
      this.push([], 0);
    };
  }

  private toggling = (event: Event): void => {
    const element = event.target;
    if (!(element instanceof HTMLElement) || !element.hasAttribute('popover') || !('newState' in event)) return;
    if (event.newState === 'open') {
      const tooltip = element.getAttribute('role') === 'tooltip';
      if (tooltip && ![...this.tracked].some(open => open.getAttribute('role') !== 'tooltip' && open.matches(':popover-open'))) return;
      if (!this.tracked.has(element)) {
        this.tracked.add(element);
        this.sizes.observe(element);
        this.moves.observe(element, { attributes: true, attributeFilter: ['style', 'class'] });
      }
      element.setAttribute(PENDING, '');
      this.waiting.set(element, 0);
      this.opened.set(element, ++this.opens);
    }
    this.schedule();
  };

  private drop(element: HTMLElement): void {
    this.tracked.delete(element);
    this.waiting.delete(element);
    this.opened.delete(element);
    this.sizes.unobserve(element);
    element.removeAttribute(PENDING);
  }

  private moved = (): void => { if (this.tracked.size) this.schedule(); };

  refresh = (): void => { if (this.tracked.size) this.schedule(); };

  private cut(element: HTMLElement, page: Rect, dpr: number): MapBounds[] {
    const order = this.opened.get(element) ?? 0;
    let parts = [physicalRect(intersectRects(element.getBoundingClientRect(), page), dpr)].filter(part => part.width > 0 && part.height > 0);
    for (const above of this.framed) {
      if (above.order < order) continue;
      for (const surface of above.surfaces) {
        const map = physicalRect(surface.getBoundingClientRect(), dpr);
        parts = parts.flatMap(part => subtract(part, map));
      }
    }
    return parts;
  }

  private schedule = (): void => {
    if (this.frame) return;
    this.frame = engine.next('measure', () => { this.frame = null; this.measure(); });
  };

  private measure(): void {
    const measured = ++this.measured;
    const dpr = window.devicePixelRatio || 1;
    const page = { left: 0, top: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight };
    const holes: MapBounds[] = [];
    const framed: Framed[] = [];
    const dimming: HTMLElement[] = [];
    const shownElements: HTMLElement[] = [];
    for (const element of [...this.tracked]) {
      const open = element.isConnected && element.matches(':popover-open');
      const rect = element.isConnected ? element.getBoundingClientRect() : null;
      if (!rect || !open && (rect.width === 0 || rect.height === 0)) { this.drop(element); continue; }
      shownElements.push(element);
      const surfaces = open ? [...element.querySelectorAll<HTMLElement>(`[${SURFACE}]`)] : [];
      if (surfaces.length) framed.push({ element, order: this.opened.get(element) ?? 0, surfaces });
      if (open && dims(element)) dimming.push(element);
    }
    this.framed = framed;
    this.watch(framed.flatMap(frame => frame.surfaces));
    for (const element of shownElements) {
      const parts = this.cut(element, page, dpr);
      if (!parts.length) continue;
      holes.push(...parts);
      if (element.matches(':popover-open') && this.waiting.has(element) && getComputedStyle(element).visibility !== 'hidden') this.waiting.set(element, measured);
    }
    dimming.sort((a, b) => (this.opened.get(a) ?? 0) - (this.opened.get(b) ?? 0));
    const windows = framed.filter(frame => frame.surfaces.some(surface => surface.getAttribute(SURFACE) !== OWN_CLOCK)).map(frame => frame.element);
    this.publish(holes, !!document.querySelector('dialog[open]'), windows, dimming);
    this.push(holes, measured);
  }

  private watch(surfaces: readonly HTMLElement[]): void {
    const now = new Set(surfaces);
    for (const surface of this.watched) if (!now.has(surface)) { this.sizes.unobserve(surface); this.watched.delete(surface); }
    for (const surface of now) if (!this.watched.has(surface)) { this.sizes.observe(surface); this.watched.add(surface); }
  }

  private publish(holes: MapBounds[], covered: boolean, windows: HTMLElement[], dimming: HTMLElement[]): void {
    const same = (next: readonly HTMLElement[], held: readonly HTMLElement[]) => next.length === held.length && next.every((element, at) => element === held[at]);
    if (covered === shown.covered && same(windows, shown.windows) && same(dimming, shown.dimming) && JSON.stringify(holes) === JSON.stringify(shown.holes)) return;
    shown.holes = holes;
    shown.covered = covered;
    shown.windows = windows;
    shown.dimming = dimming;
    for (const listener of listeners) listener();
  }

  private push(holes: MapBounds[], measured: number): void {
    this.queued = { holes, key: JSON.stringify(holes), measured };
    void this.drain();
  }

  private async drain(): Promise<void> {
    if (this.sending) return;
    this.sending = true;
    try {
      while (this.queued) {
        const next = this.queued;
        this.queued = null;
        if (next.key !== this.sent) {
          try { await this.send(next.holes); this.sent = next.key; }
          catch { this.sent = ''; this.reveal(next.measured, null); continue; }
        }
        this.reveal(next.measured, next.holes);
      }
    } finally { this.sending = false; }
  }

  private reveal(measured: number, holes: readonly MapBounds[] | null): void {
    const dpr = window.devicePixelRatio || 1;
    const page = { left: 0, top: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight };
    for (const [element, seen] of this.waiting) {
      if (seen === 0 || seen > measured) continue;
      if (holes && !this.cut(element, page, dpr).every(part => coveredBy(part, holes))) { this.schedule(); continue; }
      element.removeAttribute(PENDING);
      this.waiting.delete(element);
    }
  }
}

export function dismissForMapPress(): void {
  for (const element of document.querySelectorAll<HTMLElement>('[popover="auto"]:popover-open')) element.hidePopover();
  document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
}

export const mapHoles = {
  start(send: (holes: MapBounds[]) => Promise<void>): () => void { return new MapHoles(send).start(); },
  holes: (): readonly MapBounds[] => shown.holes,
  covered: (): boolean => shown.covered,
  outside: (surface: HTMLElement): boolean => shown.windows.length > 0 && surface.getAttribute(SURFACE) !== OWN_CLOCK && !shown.windows.some(window => window.contains(surface)),
  dimmed: (surface: HTMLElement): boolean => shown.dimming.length > 0 && !shown.dimming[shown.dimming.length - 1]!.contains(surface),
  surface(element: HTMLElement, playhead = true): () => void {
    element.setAttribute(SURFACE, playhead ? '' : OWN_CLOCK);
    active?.refresh();
    return () => { element.removeAttribute(SURFACE); active?.refresh(); };
  },
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  },
};
