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

let idlePreload = false;
let preloadDone = false;
let preloadTotal = 0;
let preloadRemaining = 0;
const preloadWaiters = new Set<() => void>();

export function preloadStatus(): { started: boolean; done: boolean; remaining: number; total: number } {
  return { started: idlePreload, done: preloadDone, remaining: preloadRemaining, total: preloadTotal };
}

export function whenPreloadComplete(): Promise<void> {
  if (preloadDone) return Promise.resolve();
  return new Promise(resolve => { preloadWaiters.add(resolve); });
}

export function preloadAllWidgetRenderers(): void {
  if (idlePreload || typeof window === 'undefined') return;
  idlePreload = true;
  const kinds = Object.keys(rendererLoaders).filter(kind => !ready.has(kind));
  preloadTotal = kinds.length;
  preloadRemaining = kinds.length;
  const finish = () => {
    preloadDone = true;
    preloadRemaining = 0;
    for (const resolve of preloadWaiters) resolve();
    preloadWaiters.clear();
  };
  if (kinds.length === 0) { finish(); return; }
  const idle = (next: () => void) => typeof requestIdleCallback === 'function' ? requestIdleCallback(next, { timeout: 2000 }) : setTimeout(next, 50);
  const step = () => {
    const kind = kinds.shift();
    if (kind === undefined) { finish(); return; }
    void loadWidgetRenderer(kind).catch(() => {}).finally(() => {
      preloadRemaining = kinds.length;
      if (kinds.length === 0) finish();
      else idle(step);
    });
  };
  idle(step);
}
