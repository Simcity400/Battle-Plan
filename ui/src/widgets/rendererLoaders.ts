import { registerWidgets } from '../dashboard/definitions.ts';
import type { WidgetRenderer } from '../dashboard/definitions.ts';
import { widgetMetas } from './definitions.ts';
import { FAMILY_VIEWS, FAMILY_VIEW_UI, LEGACY_WIDGETS } from './manifest.ts';

type RendererLoader = () => Promise<{ default: WidgetRenderer }>;

function buildRendererLoaders(): Record<string, RendererLoader> {
  const map: Record<string, RendererLoader> = {};
  for (const entry of LEGACY_WIDGETS) {
    if (!Object.hasOwn(map, entry.kind)) {
      map[entry.kind] = entry.load;
    }
  }
  for (const entry of FAMILY_VIEWS) {
    if (Object.hasOwn(map, entry.kind)) continue;
    const ui = Object.hasOwn(FAMILY_VIEW_UI, entry.kind) ? FAMILY_VIEW_UI[entry.kind] : undefined;
    map[entry.kind] = ui
      ? ui.load
      : () => entry.loadSpec().then(spec => ({ default: { declaration: spec.declaration } as WidgetRenderer }));
  }
  return map;
}

export const rendererLoaders = buildRendererLoaders() as Record<(typeof widgetMetas)[number]['kind'], RendererLoader>;

const loaded = new Map<string, Promise<WidgetRenderer>>();
const ready = new Map<string, WidgetRenderer>();
export const loadedWidgetRenderer = (kind: string): WidgetRenderer | undefined => ready.get(kind);
export function loadWidgetRenderer(kind: string): Promise<WidgetRenderer | undefined> {
  if (!Object.hasOwn(rendererLoaders, kind)) return Promise.resolve(undefined);
  const loader = rendererLoaders[kind as keyof typeof rendererLoaders];
  if (!loader) return Promise.resolve(undefined);
  let held = loaded.get(kind);
  if (!held) {
    held = loader().then(module => {
      const meta = widgetMetas.find(meta => meta.kind === kind)!;
      registerWidgets([{ ...meta, useOptionChoices: (module.default as WidgetRenderer).useOptionChoices }]);
      ready.set(kind, module.default);
      return module.default;
    });
    loaded.set(kind, held);
    held.catch(() => { loaded.delete(kind); });
  }
  return held;
}

export function preloadWidgetRenderers(kinds: Iterable<string>): void {
  for (const kind of kinds) if (!ready.has(kind)) void loadWidgetRenderer(kind).catch(() => {});
}

const activeClocks = new Set<object>();
const resumeWaiters = new Set<() => void>();

export const isPlaybackActive = (): boolean => activeClocks.size > 0;

export function setPlaybackActive(clock: object, playing: boolean): void {
  const was = activeClocks.size > 0;
  if (playing) activeClocks.add(clock);
  else activeClocks.delete(clock);
  if (was && activeClocks.size === 0) {
    for (const resume of [...resumeWaiters]) resume();
  }
}

interface IdleDeadlineLike { didTimeout: boolean; timeRemaining(): number }

export interface IdlePreloadOptions {
  chunks?: Readonly<Record<string, string>>;
  schedule?: (run: (deadline: IdleDeadlineLike) => void) => number;
  cancel?: (id: number) => void;
  preload?: (url: string) => void | Promise<void>;
  load?: (kind: string) => Promise<unknown>;
  kinds?: readonly string[];
}

const MIN_IDLE_MS = 6;

function builtChunks(): Readonly<Record<string, string>> | undefined {
  if (typeof document === 'undefined') return undefined;
  const content = document.querySelector('meta[name="bp-widget-chunks"]')?.getAttribute('content');
  if (!content) return undefined;
  try {
    const parsed = JSON.parse(content) as Record<string, string>;
    return parsed && typeof parsed === 'object' ? parsed : undefined;
  } catch {
    return undefined;
  }
}

const preloadedUrls = new Set<string>();

function preloadModuleUrl(url: string): Promise<void> {
  if (preloadedUrls.has(url) || typeof document === 'undefined') return Promise.resolve();
  preloadedUrls.add(url);
  return new Promise(resolve => {
    const link = document.createElement('link');
    link.rel = 'modulepreload';
    link.href = url;
    const done = () => { link.removeEventListener('load', done); link.removeEventListener('error', done); resolve(); };
    link.addEventListener('load', done, { once: true });
    link.addEventListener('error', done, { once: true });
    document.head.appendChild(link);
  });
}

let idlePreload = false;
let preloadQueue: string[] | null = null;
let preloadInFlight = false;
let preloadWaiters: Set<() => void> | null = null;

export function preloadStatus(): { started: boolean; done: boolean; remaining: number; total: number } {
  return {
    started: idlePreload,
    done: idlePreload && preloadQueue === null && !preloadInFlight,
    remaining: (preloadQueue ? preloadQueue.length : 0) + (preloadInFlight ? 1 : 0),
    total: Object.keys(rendererLoaders).length,
  };
}

export function whenPreloadComplete(): Promise<void> {
  if (idlePreload && preloadQueue === null && !preloadInFlight) return Promise.resolve();
  preloadWaiters ??= new Set();
  return new Promise(resolve => { preloadWaiters!.add(resolve); });
}

export function preloadAllWidgetRenderers(options?: IdlePreloadOptions): () => void {
  const custom = options !== undefined;
  if (!custom && (idlePreload || typeof window === 'undefined')) return () => {};
  if (!custom) idlePreload = true;
  const kinds = [...(options?.kinds ?? Object.keys(rendererLoaders))].filter(kind => !ready.has(kind));
  const finish = () => {
    if (custom) return;
    preloadQueue = null;
    if (preloadWaiters) {
      for (const resolve of preloadWaiters) resolve();
      preloadWaiters = null;
    }
  };
  if (kinds.length === 0) { finish(); return () => {}; }
  if (!custom) preloadQueue = kinds;
  const chunks = options?.chunks ?? builtChunks();
  const preload = options?.preload ?? preloadModuleUrl;
  const load = options?.load ?? loadWidgetRenderer;
  const schedule = options?.schedule ?? (run => typeof requestIdleCallback === 'function'
    ? requestIdleCallback(run)
    : window.setTimeout(() => run({ didTimeout: false, timeRemaining: () => MIN_IDLE_MS }), 120));
  const cancel = options?.cancel ?? (id => {
    if (typeof cancelIdleCallback === 'function') cancelIdleCallback(id);
    else clearTimeout(id);
  });
  let handle: number | null = null;
  let stopped = false;
  let inFlight = false;
  const next = () => {
    if (stopped || inFlight || handle !== null) return;
    if (!kinds.length) { finish(); return; }
    if (isPlaybackActive()) return;
    handle = schedule(deadline => {
      handle = null;
      if (stopped || isPlaybackActive()) return;
      if (deadline.didTimeout || deadline.timeRemaining() < MIN_IDLE_MS) { next(); return; }
      while (kinds.length && ready.has(kinds[0]!)) kinds.shift();
      const kind = kinds.shift();
      if (kind === undefined) { finish(); return; }
      const url = chunks?.[kind];
      inFlight = true;
      if (!custom) preloadInFlight = true;
      const pending = url ? Promise.resolve(preload(url)) : load(kind);
      void pending.catch(() => {}).finally(() => {
        inFlight = false;
        if (!custom) preloadInFlight = false;
        if (stopped) return;
        if (kinds.length === 0) finish();
        else next();
      });
    });
  };
  const onResume = () => { next(); };
  resumeWaiters.add(onResume);
  next();
  return () => {
    stopped = true;
    resumeWaiters.delete(onResume);
    if (handle !== null) { cancel(handle); handle = null; }
  };
}
