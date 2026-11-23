import { useCallback, useSyncExternalStore } from 'react';
import { errorMessage } from '../shell/api';
import type { MapBounds } from '../shell/api';
import type { MapOptions } from '../shell/api';
import type { DashboardClient, DashboardMapState, DashboardMapView, MapOptionsEvent, MapToggle } from './client';
import { PickerOptions } from './mapPicker';
import { viewsKey } from './mapPlayback';
import { ShownError } from './queryError.ts';
import type { Failure } from './queryError';
import { perfLog } from '../shell/perf';
import type { MapExpectation } from '../shell/perf';
export { CAMPAIGN_MAP_VIEW, campaignToggled, toggleMapSetting } from './mapPicker';
import { useTransitionValue } from './playback';
import { engine } from '../engine/frame.ts';
import type { PlaybackController, PlaybackSnapshot } from './playback';

const TOGGLE_NAMES: Record<MapToggle['setting'], string> = { dayNight: 'Day/night cycle', alliedPlans: 'Allied battle plans', fog: 'Fog of war' };

export type MapTarget = Omit<DashboardMapView, 'id' | 'bounds' | 'clip' | 'visible' | 'composedAt' | 'clockId'>;
interface Layout { bounds: MapBounds; clip: MapBounds; onScreen: boolean }
interface Surface { element: HTMLElement; target: (clock: PlaybackSnapshot) => MapTarget | null; layout: Layout | null; present: boolean }
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

function shownKey(state: DashboardMapState, playing: boolean): string {
  return JSON.stringify([state.sourceId, state.revision, state.branchId, state.loading, state.error ?? null,
    state.behindReason ?? null, state.hour == null, ...(playing ? [] : [state.hour, state.sequence, state.requestedSequence])]);
}

export class MapViewsStore {
  private surfaces = new Map<string, Surface>();
  private states = new Map<string, DashboardMapState>();
  private shown = new Map<string, { key: string; state: DashboardMapState }>();
  private painters = new Map<string, (state: DashboardMapState) => void>();
  private paintFrame: (() => void) | null = null;
  private listeners = new Map<string, Set<() => void>>();
  private hidden = false;
  private hiddenListeners = new Set<() => void>();
  private viewport: HTMLElement | null = null;
  private observer: ResizeObserver | null = null;
  private frame: (() => void) | null = null;
  private layoutDirty = true;
  private pending: { clockId: string; views: DashboardMapView[] } | null = null;
  private busy = false;
  private applied = '';
  private active = false;
  private generation = 0;
  private picker = new PickerOptions();
  onToggle: ((view: DashboardMapView, toggle: MapToggle) => void) | null = null;
  private lastViews = new Map<string, DashboardMapView>();
  constructor(private client: DashboardClient, private clock: PlaybackController) {}

  viewOptions(id: string, options: MapOptions | undefined): MapOptions | undefined {
    return this.picker.merge(id, options);
  }

  pickerChanged(event: MapOptionsEvent): void { this.picker.changed(event); }

  pickerToggled(viewId: string, toggle: MapToggle): void {
    if (this.picker.toggled(viewId, toggle)) return;
    const view = this.lastViews.get(viewId);
    if (view) this.onToggle?.(view, toggle);
  }

  onFog(viewId: string, flip: () => void): () => void { return this.picker.onFog(viewId, flip); }

  private toggleFailures = new Map<string, { recording: string; report: (setting: string, failure: Failure | null) => void }>();
  private toggleSaves = new Map<string, number>();
  private toggleSaved = 0;

  onToggleFailure(viewId: string, recording: string, report: (setting: string, failure: Failure | null) => void): () => void {
    const entry = { recording, report };
    this.toggleFailures.set(viewId, entry);
    return () => { if (this.toggleFailures.get(viewId) === entry) this.toggleFailures.delete(viewId); };
  }

  saveToggle(view: DashboardMapView, toggle: MapToggle, save: () => Promise<string | null>): void {
    const recording = `${view.sourceId}:${view.revision}`;
    const key = JSON.stringify([view.id, recording, toggle.setting]);
    const sequence = ++this.toggleSaved;
    this.toggleSaves.set(key, sequence);
    const tell = (failure: Failure | null) => {
      if (this.toggleSaves.get(key) !== sequence) return;
      if (!failure) this.toggleSaves.delete(key);
      const listener = this.toggleFailures.get(view.id);
      if (listener?.recording === recording) listener.report(toggle.setting, failure);
    };
    save().then(warning => tell(warning ? new ShownError(`${TOGGLE_NAMES[toggle.setting]}: changed, but Windows didn’t confirm the change reached the disk.`, warning) : null), (error: unknown) => tell(new ShownError(
      `${TOGGLE_NAMES[toggle.setting]}: this change may not have been kept. Toggling it again makes a new change.`, errorMessage(error))));
  }

  start(viewport: HTMLElement): () => void {
    this.active = true;
    this.viewport = viewport;
    const generation = ++this.generation;
    this.observer = new ResizeObserver(this.schedule);
    this.observer.observe(viewport);
    for (const surface of this.surfaces.values()) this.observer.observe(surface.element);
    const stopClock = this.clock.subscribe(this.retarget);
    const stopShown = this.clock.subscribe(this.presentAll);
    const stopFrame = this.clock.subscribeFrame(this.checkPresence);
    let stopStates: (() => void) | undefined;
    let stopOptions: (() => void) | undefined;
    let stopToggles: (() => void) | undefined;
    const alive = () => this.active && this.generation === generation;
    void this.client.onMapOptions?.((event) => {
      if (!alive()) return;
      this.pickerChanged(event);
      this.retarget();
    }).then((stop) => { if (!alive()) stop(); else stopOptions = stop; });
    void this.client.onMapToggle?.((event) => {
      if (alive()) this.pickerToggled(event.viewId, event.toggle);
    }).then((stop) => { if (!alive()) stop(); else stopToggles = stop; });
    let resolution: MediaQueryList | null = null;
    const watchResolution = () => {
      resolution?.removeEventListener('change', changedResolution);
      resolution = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      resolution.addEventListener('change', changedResolution);
    };
    const changedResolution = () => { watchResolution(); this.schedule(); };
    watchResolution();
    window.addEventListener('resize', this.schedule);
    window.addEventListener('scroll', this.scrolled, true);
    window.visualViewport?.addEventListener('resize', this.schedule);
    window.visualViewport?.addEventListener('scroll', this.schedule);
    document.addEventListener('visibilitychange', this.schedule);
    if (this.client.available) void this.client.onMapStates((states) => {
      if (!this.active || this.generation !== generation) return;
      perfLog?.mapStates(states, this.expectation);
      let errorChanged = false;
      for (const state of states) {
        if (state.clockId !== this.clock.clockId) continue;
        const old = this.states.get(state.id);
        if (old && (state.requestedSequence ?? state.sequence) < (old.requestedSequence ?? old.sequence)
          && old.sourceId === state.sourceId && old.revision === state.revision) continue;
        errorChanged ||= !old?.error !== !state.error;
        this.states.set(state.id, state);
        this.present(state.id);
      }
      this.schedulePaint();
      if (errorChanged) this.retarget();
    }).then((stop) => {
      if (!this.active || this.generation !== generation) stop(); else stopStates = stop;
    }).catch((cause) => { if (this.active && this.generation === generation) this.fail(errorMessage(cause)); });
    this.schedule();
    return () => {
      this.active = false;
      this.states.clear();
      this.shown.clear();
      this.paintFrame?.();
      this.paintFrame = null;
      stopClock();
      stopShown();
      stopFrame();
      stopStates?.();
      stopOptions?.();
      stopToggles?.();
      this.observer?.disconnect();
      this.observer = null;
      this.frame?.();
      this.frame = null;
      window.removeEventListener('resize', this.schedule);
      window.removeEventListener('scroll', this.scrolled, true);
      window.visualViewport?.removeEventListener('resize', this.schedule);
      window.visualViewport?.removeEventListener('scroll', this.schedule);
      document.removeEventListener('visibilitychange', this.schedule);
      resolution?.removeEventListener('change', changedResolution);
      this.pending = { clockId: this.clock.clockId, views: [] };
      perfLog?.mapViews([]);
      void this.drain();
    };
  }

  register(id: string, element: HTMLElement, target: Surface['target']): () => void {
    const surface: Surface = { element, target, layout: null, present: false };
    this.surfaces.set(id, surface);
    this.observer?.observe(element);
    this.schedule();
    return () => {
      if (this.surfaces.get(id) === surface) this.surfaces.delete(id);
      this.observer?.unobserve(element);
      this.schedule();
    };
  }

  setHidden(hidden: boolean): void {
    if (hidden === this.hidden) return;
    this.hidden = hidden;
    this.retarget();
    for (const listener of this.hiddenListeners) listener();
  }
  isHidden = (): boolean => this.hidden;
  subscribeHidden = (listener: () => void): (() => void) => {
    this.hiddenListeners.add(listener);
    return () => { this.hiddenListeners.delete(listener); };
  };
  retry(id: string): void {
    this.states.delete(id);
    this.shown.delete(id);
    this.applied = '';
    this.notify(id);
    this.retarget();
  }
  resend(): void { this.applied = ''; this.retarget(); }
  getState = (id: string): DashboardMapState | null => this.shown.get(id)?.state ?? null;
  paint(id: string, painter: (state: DashboardMapState) => void): () => void {
    this.painters.set(id, painter);
    const state = this.states.get(id);
    if (state) painter(state);
    return () => { if (this.painters.get(id) === painter) this.painters.delete(id); };
  }
  subscribe = (id: string, listener: () => void): (() => void) => {
    let listeners = this.listeners.get(id);
    if (!listeners) { listeners = new Set(); this.listeners.set(id, listeners); }
    listeners.add(listener);
    return () => { listeners.delete(listener); if (!listeners.size) this.listeners.delete(id); };
  };
  schedule = (): void => { this.layoutDirty = true; this.retarget(); };
  private scrolled = (event: Event): void => {
    const target = event.target as Partial<Node> | null;
    if (target && typeof target.contains === 'function' && target !== document.documentElement
      && ![...this.surfaces.values()].some(surface => target.contains!(surface.element))) return;
    this.schedule();
  };
  retarget = (): void => {
    if (!this.active || this.frame) return;
    this.frame = engine.next('measure', () => { this.frame = null; this.compose(); });
  };
  private checkPresence = (): void => {
    const clock = this.clock.getSnapshot();
    for (const surface of this.surfaces.values()) {
      if ((surface.target(clock) != null) !== surface.present) { this.retarget(); return; }
    }
  };

  private expectation = (id: string): MapExpectation | null => {
    const clock = this.clock.getSnapshot();
    const target = this.surfaces.get(id)?.target(clock);
    if (!target) return null;
    const moved = target.playing ? this.clock.positionAt() - clock.position : 0;
    return { position: target.hour + target.fraction + moved, follow: target.seekSequence !== undefined };
  };

  private notify(id: string): void { for (const listener of this.listeners.get(id) ?? []) listener(); }
  private present(id: string): void {
    const state = this.states.get(id);
    if (!state) return;
    const key = shownKey(state, this.clock.getSnapshot().playing);
    if (this.shown.get(id)?.key === key) return;
    this.shown.set(id, { key, state });
    this.notify(id);
  }
  private presentAll = (): void => { for (const id of this.states.keys()) this.present(id); };
  private schedulePaint(): void {
    if (this.paintFrame || !this.painters.size || typeof requestAnimationFrame !== 'function') return;
    this.paintFrame = engine.next('paint', () => {
      this.paintFrame = null;
      for (const [id, painter] of this.painters) {
        const state = this.states.get(id);
        if (state) painter(state);
      }
    });
  }
  private fail(error: string): void {
    let changed = false;
    for (const id of this.surfaces.keys()) {
      const old = this.states.get(id);
      changed ||= old?.error !== error;
      this.states.set(id, old ? { ...old, error } : { id, clockId: this.clock.clockId, sourceId: null, revision: null, branchId: null, hour: null,
        fraction: null, sequence: 0, requestedSequence: 0, loading: false, error });
      this.present(id);
    }
    if (changed) this.retarget();
  }

  private measure(viewport: HTMLElement): void {
    const clipRect = viewport.getBoundingClientRect();
    const page = { left: 0, top: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight };
    const dpr = window.devicePixelRatio || 1;
    for (const surface of this.surfaces.values()) {
      const rect = surface.element.getBoundingClientRect();
      const bounds = physicalRect(rect, dpr);
      const clip = physicalRect(intersectRects(rect, clipRect, page), dpr);
      surface.layout = { bounds, clip, onScreen: bounds.width > 0 && bounds.height > 0 && clip.width > 0 && clip.height > 0 };
    }
  }

  private compose(): void {
    if (!this.viewport || !this.active) return;
    if (this.layoutDirty) { this.layoutDirty = false; this.measure(this.viewport); }
    const clock = this.clock.getSnapshot();
    const composedAt = this.clock.composedAt();
    const clockId = this.clock.clockId;
    const views: DashboardMapView[] = [];
    for (const [id, surface] of this.surfaces) {
      const target = surface.target(clock);
      surface.present = target != null;
      if (!target || !surface.layout) continue;
      const { bounds, clip, onScreen } = surface.layout;
      const visible = onScreen && !this.hidden && !document.hidden && !this.states.get(id)?.error;
      const view = { id, ...target, composedAt, clockId, options: this.viewOptions(id, target.options), bounds, clip, visible };
      views.push(view);
      this.lastViews.set(id, view);
    }
    perfLog?.mapViews(views);
    this.pending = { clockId, views };
    void this.drain();
  }

  private async drain(): Promise<void> {
    if (this.busy || !this.client.available) return;
    this.busy = true;
    try {
      while (this.pending) {
        const snapshot = this.pending;
        this.pending = null;
        const key = viewsKey(snapshot.clockId, snapshot.views);
        if (key === this.applied) continue;
        try { await this.client.setMapViews(snapshot.clockId, snapshot.views); this.applied = key; }
        catch (cause) { this.applied = ''; this.fail(errorMessage(cause)); }
      }
    } finally { this.busy = false; }
  }
}

export function useMapsHidden(store: MapViewsStore): boolean {
  return useSyncExternalStore(store.subscribeHidden, store.isHidden, store.isHidden);
}

export function useMapState(store: MapViewsStore, id: string): DashboardMapState | null {
  const subscribe = useCallback((listener: () => void) => store.subscribe(id, listener), [store, id]);
  const snapshot = useCallback(() => store.getState(id), [store, id]);
  return useTransitionValue(subscribe, snapshot);
}
