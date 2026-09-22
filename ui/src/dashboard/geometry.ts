import { useCallback, useSyncExternalStore } from 'react';
import { errorMessage } from '../shell/api';
import type { MapBounds } from '../shell/api';
import type { DashboardClient, DashboardMapState, DashboardMapView } from './client';
import type { PlaybackController, PlaybackSnapshot } from './playback';

export type MapTarget = Omit<DashboardMapView, 'id' | 'bounds' | 'clip' | 'visible'>;
interface Surface { element: HTMLElement; target: (clock: PlaybackSnapshot) => MapTarget | null }
interface Rect { left: number; top: number; right: number; bottom: number }

export function intersectRects(...rects: Rect[]): Rect {
  return { left: Math.max(...rects.map((rect) => rect.left)), top: Math.max(...rects.map((rect) => rect.top)),
    right: Math.min(...rects.map((rect) => rect.right)), bottom: Math.min(...rects.map((rect) => rect.bottom)) };
}
export function physicalRect(rect: Rect, dpr: number): MapBounds {
  const x = Math.round(rect.left * dpr);
  const y = Math.round(rect.top * dpr);
  return { x, y, width: Math.max(0, Math.round(rect.right * dpr) - x), height: Math.max(0, Math.round(rect.bottom * dpr) - y) };
}

/** One batched native geometry/target channel, independently acknowledged per map. */
export class MapViewsStore {
  private surfaces = new Map<string, Surface>();
  private states = new Map<string, DashboardMapState>();
  private listeners = new Map<string, Set<() => void>>();
  private hidden = false;
  private viewport: HTMLElement | null = null;
  private observer: ResizeObserver | null = null;
  private frame = 0;
  private pending: DashboardMapView[] | null = null;
  private busy = false;
  private applied = '';
  private active = false;
  private generation = 0;
  constructor(private client: DashboardClient, private clock: PlaybackController) {}

  start(viewport: HTMLElement): () => void {
    this.active = true;
    this.viewport = viewport;
    const generation = ++this.generation;
    this.observer = new ResizeObserver(this.schedule);
    this.observer.observe(viewport);
    for (const surface of this.surfaces.values()) this.observer.observe(surface.element);
    const stopClock = this.clock.subscribe(this.schedule);
    let stopStates: (() => void) | undefined;
    let resolution: MediaQueryList | null = null;
    const watchResolution = () => {
      resolution?.removeEventListener('change', changedResolution);
      resolution = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      resolution.addEventListener('change', changedResolution);
    };
    const changedResolution = () => { watchResolution(); this.schedule(); };
    watchResolution();
    window.addEventListener('resize', this.schedule);
    window.addEventListener('scroll', this.schedule, true);
    window.visualViewport?.addEventListener('resize', this.schedule);
    window.visualViewport?.addEventListener('scroll', this.schedule);
    document.addEventListener('visibilitychange', this.schedule);
    if (this.client.available) void this.client.onMapStates((states) => {
      if (!this.active || this.generation !== generation) return;
      for (const state of states) {
        const old = this.states.get(state.id);
        if (old && (state.requestedSequence ?? state.sequence) < (old.requestedSequence ?? old.sequence)
          && old.sourceId === state.sourceId && old.revision === state.revision) continue;
        this.states.set(state.id, state);
        this.notify(state.id);
      }
      this.schedule();
    }).then((stop) => {
      if (!this.active || this.generation !== generation) stop(); else stopStates = stop;
    }).catch((cause) => { if (this.active && this.generation === generation) this.fail(errorMessage(cause)); });
    this.schedule();
    return () => {
      this.active = false;
      this.states.clear();
      stopClock();
      stopStates?.();
      this.observer?.disconnect();
      this.observer = null;
      if (this.frame) cancelAnimationFrame(this.frame);
      this.frame = 0;
      window.removeEventListener('resize', this.schedule);
      window.removeEventListener('scroll', this.schedule, true);
      window.visualViewport?.removeEventListener('resize', this.schedule);
      window.visualViewport?.removeEventListener('scroll', this.schedule);
      document.removeEventListener('visibilitychange', this.schedule);
      resolution?.removeEventListener('change', changedResolution);
      this.pending = [];
      void this.drain();
    };
  }

  register(id: string, element: HTMLElement, target: Surface['target']): () => void {
    const surface = { element, target };
    this.surfaces.set(id, surface);
    this.observer?.observe(element);
    this.schedule();
    return () => {
      if (this.surfaces.get(id) === surface) this.surfaces.delete(id);
      this.observer?.unobserve(element);
      this.schedule();
    };
  }

  setHidden(hidden: boolean): void { if (hidden !== this.hidden) { this.hidden = hidden; this.schedule(); } }
  retry(id: string): void {
    this.states.delete(id);
    this.applied = '';
    this.notify(id);
    this.schedule();
  }
  getState = (id: string): DashboardMapState | null => this.states.get(id) ?? null;
  subscribe = (id: string, listener: () => void): (() => void) => {
    let listeners = this.listeners.get(id);
    if (!listeners) { listeners = new Set(); this.listeners.set(id, listeners); }
    listeners.add(listener);
    return () => { listeners.delete(listener); if (!listeners.size) this.listeners.delete(id); };
  };
  schedule = (): void => {
    if (!this.active || this.frame) return;
    this.frame = requestAnimationFrame(() => { this.frame = 0; this.measure(); });
  };

  private notify(id: string): void { for (const listener of this.listeners.get(id) ?? []) listener(); }
  private fail(error: string): void {
    let changed = false;
    for (const id of this.surfaces.keys()) {
      const old = this.states.get(id);
      changed ||= old?.error !== error;
      this.states.set(id, old ? { ...old, error } : { id, sourceId: null, revision: null, branchId: null, hour: null,
        fraction: null, sequence: 0, requestedSequence: 0, loading: false, error });
      this.notify(id);
    }
    if (changed) this.schedule();
  }

  private measure(): void {
    if (!this.viewport || !this.active) return;
    const clipRect = this.viewport.getBoundingClientRect();
    const viewport = { left: 0, top: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight };
    const dpr = window.devicePixelRatio || 1;
    const clock = this.clock.getSnapshot();
    const views: DashboardMapView[] = [];
    for (const [id, surface] of this.surfaces) {
      const target = surface.target(clock);
      if (!target) continue;
      const rect = surface.element.getBoundingClientRect();
      const intersection = intersectRects(rect, clipRect, viewport);
      const bounds = physicalRect(rect, dpr);
      const clip = physicalRect(intersection, dpr);
      const visible = !this.hidden && !document.hidden && !this.states.get(id)?.error
        && bounds.width > 0 && bounds.height > 0 && clip.width > 0 && clip.height > 0;
      views.push({ id, ...target, bounds, clip, visible });
    }
    this.pending = views;
    void this.drain();
  }

  private async drain(): Promise<void> {
    if (this.busy || !this.client.available) return;
    this.busy = true;
    try {
      while (this.pending) {
        const views = this.pending;
        this.pending = null;
        const key = JSON.stringify(views);
        if (key === this.applied) continue;
        try { await this.client.setMapViews(views); this.applied = key; }
        catch (cause) { this.applied = ''; this.fail(errorMessage(cause)); }
      }
    } finally { this.busy = false; }
  }
}

export function useMapState(store: MapViewsStore, id: string): DashboardMapState | null {
  const subscribe = useCallback((listener: () => void) => store.subscribe(id, listener), [store, id]);
  const snapshot = useCallback(() => store.getState(id), [store, id]);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
