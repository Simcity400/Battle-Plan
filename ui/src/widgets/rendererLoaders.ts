import { registerWidgets } from '../dashboard/definitions.ts';
import type { FamilyViewRenderer, WidgetRenderer } from '../dashboard/definitions.ts';
import { widgetMetas } from './definitions.ts';
import { DEFAULT_VIEW, normalizeView, viewKey, type FamilyViewEntry, type NativeWidgetEntry } from '../families/manifest.ts';
import { FAMILY_VIEWS, FAMILY_VIEW_UI, NATIVE_WIDGETS, OPTION_CHOICES } from './manifest.ts';

type RendererLoader = () => Promise<{ default: WidgetRenderer }>;

export interface RendererLoaderEntry {
  readonly kind: string;
  readonly view: string;
  readonly load: RendererLoader;
}

export function buildRendererLoaders(
  native: readonly NativeWidgetEntry[],
  family: readonly FamilyViewEntry[],
  ui: Readonly<Record<string, { readonly load: () => Promise<{ default: FamilyViewRenderer }>; readonly views?: readonly string[] }>>,
): ReadonlyMap<string, RendererLoaderEntry> {
  const map = new Map<string, RendererLoaderEntry>();
  for (const { kind, load } of native) {
    map.set(viewKey(kind), { kind, view: DEFAULT_VIEW, load });
  }
  for (const entry of family) {
    const key = viewKey(entry.kind, entry.view);
    if (map.has(key)) continue;
    const held = Object.hasOwn(ui, entry.kind) ? ui[entry.kind] : undefined;
    const override = held && (!held.views || held.views.includes(normalizeView(entry.view))) ? held : undefined;
    map.set(key, {
      kind: entry.kind,
      view: normalizeView(entry.view),
      load: override
        ? () => Promise.all([override.load(), entry.loadSpec()])
          .then(([module, spec]) => ({ default: { ...module.default, declaration: spec.declaration } as unknown as WidgetRenderer }))
        : () => entry.loadSpec().then(spec => ({ default: { declaration: spec.declaration } as WidgetRenderer })),
    });
  }
  return map;
}

const rendererLoaders = buildRendererLoaders(NATIVE_WIDGETS, FAMILY_VIEWS, FAMILY_VIEW_UI);
const defaultViews = new Map<string, string>();
for (const [key, entry] of rendererLoaders) if (!defaultViews.has(entry.kind)) defaultViews.set(entry.kind, key);
const keyFor = (kind: string, view?: string) => view === undefined ? defaultViews.get(kind) : viewKey(kind, view);

const loaded = new Map<string, Promise<WidgetRenderer>>();
const ready = new Map<string, WidgetRenderer>();
const choiceKinds = new Set<string>();
export const loadedWidgetRenderer = (kind: string, view?: string): WidgetRenderer | undefined => {
  const key = keyFor(kind, view);
  return key === undefined ? undefined : ready.get(key);
};
export function loadWidgetRenderer(kind: string, view?: string): Promise<WidgetRenderer | undefined> {
  const key = keyFor(kind, view);
  const entry = key === undefined ? undefined : rendererLoaders.get(key);
  if (key === undefined || !entry) return Promise.resolve(undefined);
  let held = loaded.get(key);
  if (!held) {
    const choices = Object.hasOwn(OPTION_CHOICES, kind) ? OPTION_CHOICES[kind]!() : Promise.resolve(undefined);
    held = Promise.all([entry.load(), choices]).then(([module, kindChoices]) => {
      const meta = widgetMetas.find(meta => meta.kind === kind)!;
      const useOptionChoices = kindChoices ?? (module.default as WidgetRenderer).useOptionChoices;
      if (useOptionChoices && !choiceKinds.has(kind)) {
        choiceKinds.add(kind);
        registerWidgets([{ ...meta, useOptionChoices }]);
      }
      ready.set(key, module.default);
      return module.default;
    });
    loaded.set(key, held);
    held.catch(() => { loaded.delete(key); });
  }
  return held;
}

export function preloadWidgetRenderers(kinds: Iterable<string>): void {
  for (const kind of kinds) if (!loadedWidgetRenderer(kind)) void loadWidgetRenderer(kind).catch(() => {});
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
let preloadQueue: readonly unknown[] | null = null;
let preloadInFlight = false;
let preloadWaiters: Set<() => void> | null = null;

export function preloadStatus(): { started: boolean; done: boolean; remaining: number; total: number } {
  return {
    started: idlePreload,
    done: idlePreload && preloadQueue === null && !preloadInFlight,
    remaining: (preloadQueue ? preloadQueue.length : 0) + (preloadInFlight ? 1 : 0),
    total: rendererLoaders.size,
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
  const queued = (options?.kinds
    ? options.kinds.map(kind => ({ kind, view: undefined as string | undefined, key: keyFor(kind) ?? kind }))
    : [...rendererLoaders].map(([key, entry]) => ({ kind: entry.kind, view: entry.view as string | undefined, key })))
    .filter(item => !ready.has(item.key));
  const finish = () => {
    if (custom) return;
    preloadQueue = null;
    if (preloadWaiters) {
      for (const resolve of preloadWaiters) resolve();
      preloadWaiters = null;
    }
  };
  if (queued.length === 0) { finish(); return () => {}; }
  if (!custom) preloadQueue = queued;
  const chunks = options?.chunks ?? builtChunks();
  const preload = options?.preload ?? preloadModuleUrl;
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
    if (!queued.length) { finish(); return; }
    if (isPlaybackActive()) return;
    handle = schedule(deadline => {
      handle = null;
      if (stopped || isPlaybackActive()) return;
      if (deadline.didTimeout || deadline.timeRemaining() < MIN_IDLE_MS) { next(); return; }
      while (queued.length && ready.has(queued[0]!.key)) queued.shift();
      const item = queued.shift();
      if (item === undefined) { finish(); return; }
      const kind = item.kind;
      const url = chunks?.[kind];
      inFlight = true;
      if (!custom) preloadInFlight = true;
      const pending = url ? Promise.resolve(preload(url)) : (options?.load ? options.load(kind) : loadWidgetRenderer(kind, item.view));
      void pending.catch(() => {}).finally(() => {
        inFlight = false;
        if (!custom) preloadInFlight = false;
        if (stopped) return;
        if (queued.length === 0) finish();
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
