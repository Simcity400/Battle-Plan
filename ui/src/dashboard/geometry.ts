import { useCallback } from 'react';
import { errorMessage } from '../shell/api';
import type { MapBounds } from '../shell/api';
import type { MapOptions } from '../shell/api';
import type { DashboardClient, DashboardMapState, DashboardMapView, MapOptionsEvent, MapToggle } from './client';
import { PickerOptions } from './mapPicker';
import { ShownError } from './queryError.ts';
import type { Failure } from './queryError';
import { perfLog } from '../shell/perf';
import type { MapExpectation } from '../shell/perf';
export { CAMPAIGN_MAP_VIEW, campaignToggled, toggleMapSetting } from './mapPicker';
import { useTransitionValue } from './playback';
import type { PlaybackController, PlaybackSnapshot } from './playback';

/** A picker toggle as its failure names it. */
const TOGGLE_NAMES: Record<MapToggle['setting'], string> = { dayNight: 'Day/night cycle', alliedPlans: 'Allied battle plans', fog: 'Fog of war' };

export type MapTarget = Omit<DashboardMapView, 'id' | 'bounds' | 'clip' | 'visible'>;
/** Where a surface was last measured; `onScreen` when both its body and visible part have area. */
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

/** One batched native geometry/target channel, independently acknowledged per map. Layout is
 * measured only when it can have changed (`schedule`); clock anchors, a target appearing or going,
 * picker options, hiding and map errors recompose views from the measured layout (`retarget`).
 * Between anchors each native map runs its own clock, so forward playback sends nothing. */
/** What a map widget shows of an acknowledgement. Playing, any ready frame of the branch answers
 * the clock, so the frame's hour and sequences, which change with every frame drawn, are left out;
 * its date is painted, not rendered (`MapViewsStore.paint`). */
function shownKey(state: DashboardMapState, playing: boolean): string {
  return JSON.stringify([state.sourceId, state.revision, state.branchId, state.loading, state.error ?? null,
    state.behindReason ?? null, state.hour == null, ...(playing ? [] : [state.hour, state.sequence, state.requestedSequence])]);
}

export class MapViewsStore {
  private surfaces = new Map<string, Surface>();
  /** Each view's newest acknowledgement. */
  private states = new Map<string, DashboardMapState>();
  /** The acknowledgement each view's widget renders, replaced only when what it shows changes. */
  private shown = new Map<string, { key: string; state: DashboardMapState }>();
  private painters = new Map<string, (state: DashboardMapState) => void>();
  private paintFrame = 0;
  private listeners = new Map<string, Set<() => void>>();
  private hidden = false;
  private viewport: HTMLElement | null = null;
  private observer: ResizeObserver | null = null;
  private frame = 0;
  private layoutDirty = true;
  private pending: DashboardMapView[] | null = null;
  private busy = false;
  private applied = '';
  private active = false;
  private generation = 0;
  /** What each view's native picker chose, and where its fog toggle goes. */
  private picker = new PickerOptions();
  /** Applies a picker toggle the host owns to a view's settings. */
  onToggle: ((view: DashboardMapView, toggle: MapToggle) => void) | null = null;
  private lastViews = new Map<string, DashboardMapView>();
  constructor(private client: DashboardClient, private clock: PlaybackController) {}

  /** The options a view shows: the target's, with the map mode and slots its picker set. */
  viewOptions(id: string, options: MapOptions | undefined): MapOptions | undefined {
    return this.picker.merge(id, options);
  }

  /** Keeps what a view's picker chose for its next layouts. */
  pickerChanged(event: MapOptionsEvent): void { this.picker.changed(event); }

  /** Routes a picker toggle: fog to the view's own switch, the rest to the host's settings. */
  pickerToggled(viewId: string, toggle: MapToggle): void {
    if (this.picker.toggled(viewId, toggle)) return;
    const view = this.lastViews.get(viewId);
    if (view) this.onToggle?.(view, toggle);
  }

  /** Lets a view's picker set its fog switch. */
  onFog(viewId: string, flip: () => void): () => void { return this.picker.onFog(viewId, flip); }

  /** Where each view's failed picker toggles are told, for the recording it shows; the latest save of each setting. */
  private toggleFailures = new Map<string, { recording: string; report: (setting: string, failure: Failure | null) => void }>();
  private toggleSaves = new Map<string, number>();
  private toggleSaved = 0;

  /** Lets a view showing `recording` (`sourceId:revision`) show its picker toggles that failed to save, by
   * setting: null once that setting's latest toggle has saved. Toggles made on another recording are never told. */
  onToggleFailure(viewId: string, recording: string, report: (setting: string, failure: Failure | null) => void): () => void {
    const entry = { recording, report };
    this.toggleFailures.set(viewId, entry);
    return () => { if (this.toggleFailures.get(viewId) === entry) this.toggleFailures.delete(viewId); };
  }

  /** Saves a view's picker toggle with `save`, telling the view whether the setting's latest toggle saved. A toggle
   * flips the saved choice, so a failed one is never sent again: toggling again is the user's own new flip. */
  saveToggle(view: DashboardMapView, toggle: MapToggle, save: () => Promise<void>): void {
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
    // The service can keep a flip and still fail to answer, so the notice never says it was lost.
    save().then(() => tell(null), (error: unknown) => tell(new ShownError(
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
    // Playing or paused decides what of an acknowledgement is shown.
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
        const old = this.states.get(state.id);
        if (old && (state.requestedSequence ?? state.sequence) < (old.requestedSequence ?? old.sequence)
          && old.sourceId === state.sourceId && old.revision === state.revision) continue;
        // A view's visibility depends only on whether its map failed.
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
      if (this.paintFrame) cancelAnimationFrame(this.paintFrame);
      this.paintFrame = 0;
      stopClock();
      stopShown();
      stopFrame();
      stopStates?.();
      stopOptions?.();
      stopToggles?.();
      this.observer?.disconnect();
      this.observer = null;
      if (this.frame) cancelAnimationFrame(this.frame);
      this.frame = 0;
      window.removeEventListener('resize', this.schedule);
      window.removeEventListener('scroll', this.scrolled, true);
      window.visualViewport?.removeEventListener('resize', this.schedule);
      window.visualViewport?.removeEventListener('scroll', this.schedule);
      document.removeEventListener('visibilitychange', this.schedule);
      resolution?.removeEventListener('change', changedResolution);
      this.pending = [];
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

  setHidden(hidden: boolean): void { if (hidden !== this.hidden) { this.hidden = hidden; this.retarget(); } }
  retry(id: string): void {
    this.states.delete(id);
    this.shown.delete(id);
    this.applied = '';
    this.notify(id);
    this.retarget();
  }
  /** Sends the views again, unchanged or not: a restarted viewer is restored with the last anchor it was sent. */
  resend(): void { this.applied = ''; this.retarget(); }
  /** The acknowledgement a view's widget renders ([`shownKey`]). */
  getState = (id: string): DashboardMapState | null => this.shown.get(id)?.state ?? null;
  /** Paints a view's newest acknowledgement now and at most once a frame after acknowledgements arrive:
   * what changes with every frame the map draws reaches the page this way, never through a render. */
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
  /** Layout may have changed: measure again before the next views go out. */
  schedule = (): void => { this.layoutDirty = true; this.retarget(); };
  /** A scroll moves a map only from the page or an element holding one: a widget's own list or
   * table scrolling as playback reveals rows must not force the page's layout to be measured. */
  private scrolled = (event: Event): void => {
    const target = event.target as Partial<Node> | null;
    if (target && typeof target.contains === 'function' && target !== document.documentElement
      && ![...this.surfaces.values()].some(surface => target.contains!(surface.element))) return;
    this.schedule();
  };
  /** Views may have changed but layout has not. */
  retarget = (): void => {
    if (!this.active || this.frame) return;
    this.frame = requestAnimationFrame(() => { this.frame = 0; this.compose(); });
  };
  /** A followed target appears or goes as the clock crosses its recording's ends. */
  private checkPresence = (): void => {
    const clock = this.clock.getSnapshot();
    for (const surface of this.surfaces.values()) {
      if ((surface.target(clock) != null) !== surface.present) { this.retarget(); return; }
    }
  };

  /** Where a map should be now in its own recording's hours: a followed map moves with the clock, a pinned one stays. */
  private expectation = (id: string): MapExpectation | null => {
    const clock = this.clock.getSnapshot();
    const target = this.surfaces.get(id)?.target(clock);
    if (!target) return null;
    const moved = target.playing ? this.clock.positionAt() - clock.position : 0;
    return { position: target.hour + target.fraction + moved, follow: target.seekSequence !== undefined };
  };

  private notify(id: string): void { for (const listener of this.listeners.get(id) ?? []) listener(); }
  /** Tells a view's widget only when what it shows of the newest acknowledgement changed. */
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
    this.paintFrame = requestAnimationFrame(() => {
      this.paintFrame = 0;
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
      this.states.set(id, old ? { ...old, error } : { id, sourceId: null, revision: null, branchId: null, hour: null,
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
    const views: DashboardMapView[] = [];
    for (const [id, surface] of this.surfaces) {
      const target = surface.target(clock);
      surface.present = target != null;
      if (!target || !surface.layout) continue;
      const { bounds, clip, onScreen } = surface.layout;
      const visible = onScreen && !this.hidden && !document.hidden && !this.states.get(id)?.error;
      const view = { id, ...target, options: this.viewOptions(id, target.options), bounds, clip, visible };
      views.push(view);
      this.lastViews.set(id, view);
    }
    perfLog?.mapViews(views);
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

/** What a map's widget shows of its acknowledged state ([`MapViewsStore.getState`]), as a transition. */
export function useMapState(store: MapViewsStore, id: string): DashboardMapState | null {
  const subscribe = useCallback((listener: () => void) => store.subscribe(id, listener), [store, id]);
  const snapshot = useCallback(() => store.getState(id), [store, id]);
  return useTransitionValue(subscribe, snapshot);
}
