import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ReactElement } from 'react';

type Element = ReactElement<Record<string, any>>;
const elements = (value: any): Element[] => Array.isArray(value) ? value.flatMap(elements)
  : value?.props ? [value, ...elements(value.props.children)] : [];
const settle = () => new Promise(resolve => setImmediate(resolve));
/** A campaign UUID header's campaign key: the id's bytes in hex. */
const CAMPAIGN_KEY = [...'00000000-0000-4000-8000-000000000001'].map(character => character.charCodeAt(0).toString(16).padStart(2, '0')).join('');

/** The dashboard with one recording open, its source handed out by `sources`. */
async function dashboard(run: (view: any) => Promise<void>) {
  const slots: any[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  const opened: string[] = [];
  let releases = 0;
  const source = (path: string, revision: string) => ({ id: `lease-${revision}`, path, revision, metadata: {
    path, name: path.split('/').pop(), campaignKey: CAMPAIGN_KEY, branches: [], countries: [], playerCountryId: null,
  } });
  let next = source('C:/recordings/Germany 1936.battleplan', '1');
  const storage = new Map<string, string>();
  const fixture = {
    useState(initial: any) { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }]; },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useCallback(callback: any) { return callback; },
    useMemo(factory: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (!before || !deps.every((value, index) => Object.is(value, before.deps[index]))) slots[index] = { deps, value: factory() };
      return slots[index].value; },
    useEffect(effect: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (before && deps.every((value, index) => Object.is(value, before[index]))) return;
      slots[index] = deps; effects.push(effect); },
    useLayoutEffect() {},
  };
  const host = globalThis as typeof globalThis & { __renameFixture?: typeof fixture };
  host.__renameFixture = fixture;
  const replacements: Record<string, string> = {
    '/widgets/index.ts': 'import "./definitions.ts";',
    '/dashboard/geometry.ts': 'export class MapViewsStore { schedule() {} setHidden() {} }',
    '/components/Popover.tsx': 'export const usePopover = () => ({ open: false, popup: { current: null }, trigger: { current: null } }); export const Popover = () => null;',
    '/shell/api.ts': 'export const desktopAvailable = false; export const errorMessage = String; export const recordingName = path => path.split("/").pop(); export const seekMap = () => {}; export const mapAction = () => {};',
    '/shell/popout.ts': 'export const usePopout = () => ({ generation: 0, sequence: 0, viewer: 0, open: false, status: null, error: null }); export const popoutClockKey = () => null; export const popoutConfigKey = () => null; export const popoutProblem = () => null; export const popoutStopped = () => false; export const popoutUnloaded = async () => {}; export const seekPopout = async () => {}; export const configurePopout = async () => {};',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'recording-rename-fixture', enforce: 'pre',
      transform(code, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/dashboard/Dashboard.tsx') || file.endsWith('/dashboard/sources.ts')) return code
          .replace(/import \{[^\n]+\} from 'react';/, 'const { useState, useRef, useCallback, useMemo, useEffect, useLayoutEffect } = globalThis.__renameFixture;')
          .replace("import { createPortal } from 'react-dom';", 'const createPortal = value => value;');
        return Object.entries(replacements).find(([suffix]) => file.endsWith(suffix))?.[1];
      },
    }] });
  // The recording poll runs when the test says, never on a clock.
  const polls: (() => void)[] = [];
  const intervalBefore = globalThis.setInterval;
  globalThis.setInterval = ((poll: () => void) => { polls.push(poll); return polls.length; }) as unknown as typeof setInterval;
  const documentBefore = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const storageBefore = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { addEventListener() {}, removeEventListener() {}, hidden: false } });
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => { storage.set(key, value); }, removeItem: (key: string) => { storage.delete(key); } } });
  try {
    const { default: Dashboard } = await vite.ssrLoadModule('/ui/src/dashboard/Dashboard.tsx');
    const { loadAircraftChoice, saveAircraftChoice } = await vite.ssrLoadModule('/ui/src/dashboard/aircraftChoice.ts');
    const { recordingKey } = await vite.ssrLoadModule('/ui/src/dashboard/campaigns.ts');
    let located: string | null = null;
    let refreshed = 0;
    let metadata: any = null;
    const props: any = { page: 'overview', resetToken: 0, path: null, client: { available: true,
      openSource: async (path: string) => { opened.push(path); return next; }, releaseSource: async () => { releases++; },
      locateRecordings: async (shown: { path: string }[]) => shown.map(() => located),
      product: async () => { throw new Error('unexpected product'); } },
      library: { files: [], refresh: async () => { refreshed++; } }, campaignHost: {}, playbackHost: {}, active: true,
      onMetadata: (value: any) => { metadata = value; },
    };
    let nodes: Element[] = [];
    const render = () => { cursor = 0; effects = []; nodes = elements(Dashboard(props)); for (const effect of effects) effect(); };
    const sidebar = () => { const node = nodes.find(node => 'countryTags' in node.props && 'onLoad' in node.props); assert.ok(node); return node; };
    const layout = () => { const node = nodes.find(node => 'runtime' in node.props && 'onLayouts' in node.props); assert.ok(node); return node.props.template; };
    const reconnect = async () => {
      const node = nodes.find(node => typeof node.props.onReconnect === 'function'); assert.ok(node);
      node.props.onReconnect(); render(); await settle(); render(); render();
    };
    render(); sidebar().props.onLoad(next.path, 'Germany 1936.battleplan', 'new');
    await settle(); render(); render();
    saveAircraftChoice(recordingKey(next.path, CAMPAIGN_KEY), 'jet_mesh');
    // Each render's effect replaces the interval before it, as React clears it.
    const poll = async () => { polls.at(-1)?.(); await settle(); render(); render(); };
    await run({ props, render, sidebar, layout, reconnect, poll, opened, releases: () => releases, storage, recordingKey, loadAircraftChoice,
      metadata: () => metadata, refreshed: () => refreshed, locate: (path: string | null) => { located = path; },
      serve: (path: string, revision: string) => { next = source(path, revision); } });
  } finally {
    globalThis.setInterval = intervalBefore;
    await vite.close(); delete host.__renameFixture;
    if (documentBefore) Object.defineProperty(globalThis, 'document', documentBefore); else Reflect.deleteProperty(globalThis, 'document');
    if (storageBefore) Object.defineProperty(globalThis, 'localStorage', storageBefore); else Reflect.deleteProperty(globalThis, 'localStorage');
  }
}

/** What the dashboard shows of its campaign's recording. */
function shown(view: any) {
  const campaign = view.sidebar().props.campaigns[0];
  return { path: campaign.path, source: view.sidebar().props.bindings[campaign.id].source.path, metadata: view.metadata()?.path, loading: campaign.loading };
}

test("renaming an open recording's file reopens its source there once, the layout untouched", () => dashboard(async view => {
  const before = structuredClone(view.layout());
  const id = view.sidebar().props.campaigns[0].id;
  const barbarossa = 'C:/recordings/Barbarossa.battleplan';
  view.serve(barbarossa, '2');
  view.sidebar().props.onRenamed('C:/recordings/Germany 1936.battleplan', barbarossa);
  view.render(); await settle(); view.render(); view.render();
  assert.deepEqual(view.opened, ['C:/recordings/Germany 1936.battleplan', barbarossa], 'the native source holds the new path');
  assert.equal(view.releases(), 1);
  assert.equal(view.sidebar().props.campaigns[0].id, id);
  assert.deepEqual(shown(view), { path: barbarossa, source: barbarossa, metadata: barbarossa, loading: false });
  // Kept by its folder and campaign, its aircraft choice is the same recording's.
  assert.equal(view.loadAircraftChoice(view.recordingKey(barbarossa, CAMPAIGN_KEY)), 'jet_mesh');
  assert.deepEqual(view.layout(), before, 'the authored layout keeps no path');
  // A rename of a recording that is not open changes nothing here.
  view.sidebar().props.onRenamed('C:/recordings/Italy 1936.battleplan', 'C:/recordings/Italy 1936–1937.battleplan');
  view.render();
  assert.equal(shown(view).path, barbarossa);
}));

test('a recording Battle Plan Recorder renames while open shows its new name, and opens by it again', () => dashboard(async view => {
  // Found in place, nothing changes.
  view.locate('C:/recordings/Germany 1936.battleplan');
  await view.poll();
  assert.equal(view.refreshed(), 0);
  // Saved under a new name: found again by its folder and campaign.
  const renamed = 'C:/recordings/Germany 1936–1937.battleplan';
  view.locate(renamed);
  view.serve(renamed, '2');
  await view.poll();
  await settle(); view.render(); view.render();
  assert.deepEqual(shown(view), { path: renamed, source: renamed, metadata: renamed, loading: false });
  assert.equal(view.refreshed(), 1, 'the library lists it by its new name');
  assert.deepEqual(view.opened, ['C:/recordings/Germany 1936.battleplan', renamed], 'reopened once by its new name');
  // Its new hours open by its name now.
  await view.reconnect();
  assert.equal(view.opened.at(-1), renamed);
  // Renamed again before the reconnect found it: found first, then opened.
  const again = 'C:/recordings/Germany 1936–1938.battleplan';
  view.locate(again);
  view.serve(again, '3');
  await view.reconnect();
  assert.equal(view.opened.at(-1), again);
  assert.equal(shown(view).path, again);
}));
