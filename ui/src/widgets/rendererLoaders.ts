import { registerWidgets } from '../dashboard/definitions.ts';
import type { WidgetRenderer } from '../dashboard/definitions.ts';
import { widgetMetas } from './definitions.ts';
export const rendererLoaders = {
  'campaign-clock': () => import('./campaign-clock'),
  'apm-graph': () => import('./apm-graph'),
  'apm-table': () => import('./apm-table'),
  'war-goals': () => import('./war-goals'),
  'next-elections': () => import('./next-elections'),
  'balance-of-power': () => import('./balance-of-power'),
  'governments-in-exile': () => import('./governments-in-exile'),
  'civil-wars': () => import('./civil-wars'),
  'event-choices': () => import('./event-choices'),
  'decisions': () => import('./decisions'),
  'map': () => import('./map'),
  'national-focus': () => import('./national-focus'),
  'research': () => import('./research'),
  'active-research': () => import('./active-research'),
  'political-power': () => import('./political-power'),
  'political-power-spending': () => import('./political-power-spending'),
  'political-power-spending-breakdown': () => import('./political-power-spending-breakdown'),
  'command-power': () => import('./command-power'),
  'command-power-spending': () => import('./command-power-spending'),
  'stability': () => import('./stability'),
  'idea-timeline': () => import('./idea-timeline'),
  'national-spirits': () => import('./national-spirits'),
  'spirit-history': () => import('./spirit-history'),
  'ruling-party': () => import('./ruling-party'),
  'resource-balance': () => import('./resource-balance'),
  'equipment-stockpile': () => import('./equipment-stockpile'),
  'fuel': () => import('./fuel'),
  'production-lines': () => import('./production-lines'),
  'factories': () => import('./factories'),
  'territory': () => import('./territory'),
  'territory-breakdown': () => import('./territory-breakdown'),
  'equipment-losses': () => import('./equipment-losses'),
  'equipment-produced': () => import('./equipment-produced'),
  'production-spending': () => import('./production-spending'),
  'dockyard-spending': () => import('./dockyard-spending'),
  'military-factory-spending': () => import('./military-factory-spending'),
  'occupied-states': () => import('./occupied-states'),
  'occupation-trend': () => import('./occupation-trend'),
  'air-force-size': () => import('./air-force-size'),
  'navy-size': () => import('./navy-size'),
  'air-superiority-region': () => import('./air-superiority-region'),
  'air-superiority-timeline': () => import('./air-superiority-timeline'),
  'refits': () => import('./refits'),
  'dockyards-and-repairs': () => import('./dockyards-and-repairs'),
  'minefields': () => import('./minefields'),
  'naval-supremacy': () => import('./naval-supremacy'),
  'weather-by-region': () => import('./weather-by-region'),
  'occupation-reserve-manpower': () => import('./occupation-reserve-manpower'),
  'intel-network-coverage': () => import('./intel-network-coverage'),
  'doctrine-mastery': () => import('./doctrine-mastery'),
  'air-statistics': () => import('./air-statistics'),
  'air-missions': () => import('./air-missions'),
  'air-kills-losses': () => import('./air-kills-losses'),
  'bombing-damage': () => import('./bombing-damage'),
  'bombing-damage-breakdown': () => import('./bombing-damage-breakdown'),
  'experience': () => import('./experience'),
  'factions': () => import('./factions'),
  'faction-strength': () => import('./faction-strength'),
  'subjects': () => import('./subjects'),
  'trade-flows': () => import('./trade-flows'),
  'fuel-branches': () => import('./fuel-branches'),
  'lend-lease-balance': () => import('./lend-lease-balance'),
  'lend-lease-agreements': () => import('./lend-lease-agreements'),
  'international-market': () => import('./international-market'),
  'market-ledger': () => import('./market-ledger'),
  'production-licences': () => import('./production-licences'),
  'industrial-organisations': () => import('./industrial-organisations'),
  'equipment-designs': () => import('./equipment-designs'),
  'convoy-health': () => import('./convoy-health'),
  'peace-deals': () => import('./peace-deals'),
  'territory-transfers': () => import('./territory-transfers'),
  'wars': () => import('./wars'),
  'world-tension': () => import('./world-tension'),
  'autonomy': () => import('./autonomy'),
  'construction-log': () => import('./construction-log'),
  'construction-spending': () => import('./construction-spending'),
  'special-projects': () => import('./special-projects'),
  'nuclear-arsenal': () => import('./nuclear-arsenal'),
  'naval-losses': () => import('./naval-losses'),
  'ships-sunk': () => import('./ships-sunk'),
  'raid-log': () => import('./raid-log'),
  'raid-tally': () => import('./raid-tally'),
  'decryption': () => import('./decryption'),
  'espionage-log': () => import('./espionage-log'),
  'agency-comparison': () => import('./agency-comparison'),
  'operatives-roster': () => import('./operatives-roster'),
  'who-spies-on-whom': () => import('./who-spies-on-whom'),
  'scoreboard': () => import('./scoreboard'),
  'game-score': () => import('./game-score'),
  'casualties': () => import('./casualties'),
  'casualties-between': () => import('./casualties-between'),
  'casualties-breakdown': () => import('./casualties-breakdown'),
  'generals': () => import('./generals'),
  'general-career': () => import('./general-career'),
  'division-template': () => import('./division-template'),
  'training-queue': () => import('./training-queue'),
  'most-decorated-divisions': () => import('./most-decorated-divisions'),
  'division-service-record': () => import('./division-service-record'),
  'army-losses-by-month': () => import('./army-losses-by-month'),
  'division-equipment': () => import('./division-equipment'),
  'aces': () => import('./aces'),
  'army-size': () => import('./army-size'),
  'supply-health': () => import('./supply-health'),
  'division-log': () => import('./division-log'),
  'land-battles': () => import('./land-battles'),
  'naval-battles': () => import('./naval-battles'),
  'event-feed': () => import('./event-feed'),
  'volunteers': () => import('./volunteers'),
  'faction-upgrades': () => import('./faction-upgrades'),
  'diplomatic-pacts': () => import('./diplomatic-pacts'),
  'diplomatic-relations': () => import('./diplomatic-relations'),
  'faction-pooled-strength': () => import('./faction-pooled-strength'),
  'faction-goals': () => import('./faction-goals'),
  'war-contribution': () => import('./war-contribution'),
} satisfies Record<(typeof widgetMetas)[number]['kind'], () => Promise<{ default: WidgetRenderer }>>;
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
  preload?: (url: string) => void;
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

function preloadModuleUrl(url: string): void {
  if (preloadedUrls.has(url) || typeof document === 'undefined') return;
  preloadedUrls.add(url);
  const link = document.createElement('link');
  link.rel = 'modulepreload';
  link.href = url;
  document.head.appendChild(link);
}

let idlePreload = false;
let preloadQueue: string[] | null = null;
let preloadWaiters: Set<() => void> | null = null;

export function preloadStatus(): { started: boolean; done: boolean; remaining: number; total: number } {
  return {
    started: idlePreload,
    done: idlePreload && preloadQueue === null,
    remaining: preloadQueue ? preloadQueue.length : 0,
    total: Object.keys(rendererLoaders).length,
  };
}

export function whenPreloadComplete(): Promise<void> {
  if (idlePreload && preloadQueue === null) return Promise.resolve();
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
      if (url) {
        preload(url);
        if (kinds.length === 0) finish();
        else next();
        return;
      }
      inFlight = true;
      void load(kind).catch(() => {}).finally(() => {
        inFlight = false;
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
