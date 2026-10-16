import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ReactElement } from 'react';

type Element = ReactElement<Record<string, any>>;
const elements = (value: any): Element[] => Array.isArray(value) ? value.flatMap(elements)
  : value?.props ? [value, ...elements(value.props.children)] : [];

test('display names leave dashboard campaign paths, source metadata and layout templates unchanged', async () => {
  const slots: any[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  const source = { id: 'lease-1', path: 'C:/Old.battleplan', revision: 'revision-1', metadata: {
    path: 'C:/Old.battleplan', name: 'Old.battleplan', campaignKey: 'ab', branches: [], countries: [], playerCountryId: null,
  } };
  let opens = 0;
  let releases = 0;
  let nextSource = source;
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
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/dashboard/Dashboard.tsx') || file.endsWith('/dashboard/sources.ts')) return source
          .replace(/import \{[^\n]+\} from 'react';/, 'const { useState, useRef, useCallback, useMemo, useEffect, useLayoutEffect } = globalThis.__renameFixture;')
          .replace("import { createPortal } from 'react-dom';", 'const createPortal = value => value;');
        return Object.entries(replacements).find(([suffix]) => file.endsWith(suffix))?.[1];
      },
    }] });
  const documentBefore = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { addEventListener() {}, removeEventListener() {}, hidden: false } });
  try {
    const { default: Dashboard } = await vite.ssrLoadModule('/ui/src/dashboard/Dashboard.tsx');
    let metadata: any = null;
    const props: any = { page: 'overview', resetToken: 0, path: null, client: { available: true,
      openSource: async () => { opens++; return nextSource; }, releaseSource: async () => { releases++; }, product: async () => { throw new Error('unexpected product'); } },
      library: { files: [] }, campaignHost: {}, playbackHost: {}, active: true, onMetadata: (value: any) => { metadata = value; },
    };
    let nodes: Element[] = [];
    const render = () => { cursor = 0; effects = []; nodes = elements(Dashboard(props)); for (const effect of effects) effect(); };
    const sidebar = () => { const node = nodes.find(node => 'countryTags' in node.props && 'onLoad' in node.props); assert.ok(node); return node; };
    const layout = () => { const node = nodes.find(node => 'runtime' in node.props && 'onLayouts' in node.props); assert.ok(node); return node.props.template; };
    render(); sidebar().props.onLoad(source.path, 'Old.battleplan', 'new');
    await new Promise(resolve => setImmediate(resolve)); render(); render();
    assert.equal(opens, 1);
    assert.equal(metadata.name, 'Old.battleplan');
    const before = structuredClone(layout());
    const { saveRecordingNames, setRecordingName } = await vite.ssrLoadModule('/ui/src/dashboard/recordingNames.ts');
    saveRecordingNames(setRecordingName({}, 'c:\\OLD.battleplan', 'Weekend campaign'), { setItem() {} }); render(); render();
    assert.deepEqual(sidebar().props.campaigns[0], { id: sidebar().props.campaigns[0].id, path: 'C:/Old.battleplan', label: 'Old.battleplan', loading: false });
    const binding = sidebar().props.bindings[sidebar().props.campaigns[0].id].source;
    assert.equal(binding.id, source.id); assert.equal(binding.revision, source.revision);
    assert.equal(binding.path, 'C:/Old.battleplan');
    assert.equal(metadata.path, 'C:/Old.battleplan'); assert.equal(metadata.name, 'Old.battleplan');
    assert.deepEqual(layout(), before);
    assert.ok(!JSON.stringify(layout()).includes('Weekend campaign'));
    assert.ok(!('onRenamed' in sidebar().props));
    assert.equal(opens, 1); assert.equal(releases, 0);
    const campaignId = sidebar().props.campaigns[0].id;
    nextSource = { ...source, id: 'lease-2', revision: 'revision-2' };
    const reconnect = nodes.find(node => typeof node.props.onReconnect === 'function');
    assert.ok(reconnect);
    reconnect.props.onReconnect(); render();
    await new Promise(resolve => setImmediate(resolve)); render(); render();
    assert.equal(opens, 2); assert.equal(releases, 1, 'reconnect releases the earlier source');
    assert.equal(sidebar().props.campaigns[0].id, campaignId, 'the loaded campaign keeps its slot');
    assert.equal(sidebar().props.bindings[campaignId].source.revision, 'revision-2');
    assert.deepEqual(layout(), before, 'refreshing recording bytes leaves the authored layout intact');
  } finally {
    await vite.close(); delete host.__renameFixture;
    if (documentBefore) Object.defineProperty(globalThis, 'document', documentBefore); else Reflect.deleteProperty(globalThis, 'document');
  }
});
