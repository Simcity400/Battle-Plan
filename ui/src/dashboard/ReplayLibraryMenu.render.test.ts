import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ReactElement } from 'react';
import type { MenuItem } from '../components/ContextMenu';
import type { CampaignSettings } from './campaignSettings';

type Element = ReactElement<Record<string, any>>;
function elements(value: any): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value?.props ? [value, ...elements(value.props.children)] : [];
}
const settle = () => new Promise(resolve => setImmediate(resolve));

async function fixture(run: (view: any) => Promise<void>) {
  const slots: any[] = [];
  let cursor = 0;
  let items: MenuItem[] = [];
  let opened = 0;
  let refreshed = 0;
  let chosen = 0;
  let failure = false;
  const renames: [string, string][] = [];
  const updates: [string, string][] = [];
  const loads: any[][] = [];
  const storage = new Map<string, string>();
  const source = { id: 'lease-1', revision: 'revision-1', path: 'C:/recordings/Loaded.battleplan', metadata: {
    campaignKey: 'ab', countries: [{ id: '1', tag: 'shown-country', name: 'Shown' }, { id: '2', tag: 'mod-country-long-tag', name: 'Mod country' }],
    playerCountryId: '1', branches: [],
  } };
  const state = {
    settings: { version: 1, campaigns: { ab: { factions: { 3: { colour: '#123456' } } } }, shared: {}, sets: [] } as CampaignSettings,
    useState(initial: any) { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }]; },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useEffect(effect: () => unknown, deps?: unknown[]) { const index = cursor++; const before = slots[index];
      if (before && deps && deps.every((value, at) => Object.is(value, before[at]))) return;
      slots[index] = deps ?? []; effect(); },
    menu(next: MenuItem[]) { items = next; },
    openFlag() { opened++; },
    save(settings: CampaignSettings) { state.settings = settings; return null; },
  };
  const host = globalThis as typeof globalThis & { __campaignMenu?: typeof state };
  host.__campaignMenu = state;
  const storageBefore = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => { if (failure) throw new Error('full'); storage.set(key, value); },
  } });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'campaign-menu-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/dashboard/ReplayLibraryMenu.tsx')) return source.replace("import { useEffect, useRef, useState } from 'react';",
          'const { useEffect, useRef, useState } = globalThis.__campaignMenu;');
        if (file.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ menu: null, open: (_x, _y, items) => globalThis.__campaignMenu.menu(items) });';
        if (file.endsWith('/components/Popover.tsx')) return `export const usePopover = () => ({ open: true, trigger: { current: null }, openAt: () => globalThis.__campaignMenu.openFlag(), close() {} });
          export const Popover = props => props.children; export const PopoverHeading = () => null;`;
        if (file.endsWith('/components/CountryPicker.tsx')) return 'export default function CountryPicker() {}';
        if (file.endsWith('/dashboard/campaignSettings.ts')) return source
          .replace("import { useSyncExternalStore } from 'react';", 'const useSyncExternalStore = () => globalThis.__campaignMenu.settings;')
          .replace('return current ??= loadCampaignSettings(storage);', 'return globalThis.__campaignMenu.settings;')
          .replace('const safe = parseCampaignSettings(settings);', 'return globalThis.__campaignMenu.save(parseCampaignSettings(settings)); const safe = parseCampaignSettings(settings);');
        if (file.endsWith('/dashboard/campaignFlags.ts')) return source.replace("import { useMemo, useSyncExternalStore } from 'react';",
          'const useSyncExternalStore = (_subscribe, snapshot) => snapshot(); const useMemo = compute => compute();');
      },
    }] });
  try {
    const { default: Menu } = await vite.ssrLoadModule('/ui/src/dashboard/ReplayLibraryMenu.tsx');
    const props: any = {
      library: { files: [{ path: 'C:/recordings/Archive.battleplan', name: 'Archive', folder: '' }], loading: false, refresh: async () => { refreshed++; } },
      client: { chooseRecording: async () => 'C:/elsewhere/Picked.battleplan', renameRecording: async (path: string, name: string) => {
        renames.push([path, name]); if (failure) throw new Error('This recording is in use.');
        return path.replace(/[^/]+$/, `${name}.battleplan`);
      } },
      campaigns: [{ id: 'primary', path: source.path, label: 'Loaded.battleplan' }], bindings: { primary: { source } },
      defaultId: 'primary', countryTags: { primary: 'shown-country' }, slots: { empty: [] }, errors: {},
      onRenamed: (oldPath: string, newPath: string) => { updates.push([oldPath, newPath]); }, relocations: [],
      onLoad: (...args: any[]) => loads.push(args), onUnload: () => { chosen++; }, onDefault() {},
    };
    let nodes: Element[] = [];
    const render = () => { cursor = 0; nodes = elements(Menu(props)); };
    const find = (predicate: (node: Element) => boolean) => { const found = nodes.find(predicate); assert.ok(found); return found; };
    const loaded = () => find(node => node.props.className === 'sidebar-row campaign-row');
    const archived = () => find(node => node.props['data-tooltip']?.includes('Archive.battleplan'));
    const input = () => find(node => node.props['aria-label'] === 'Recording name');
    const event = (_button: Element, key?: string, shiftKey = false) => ({ key, shiftKey, clientX: 10, clientY: 20, preventDefault() {}, stopPropagation() {},
      currentTarget: { getBoundingClientRect: () => ({ left: 10, bottom: 20 }) } });
    const menu = (button = loaded()) => { button.props.onContextMenu(event(button)); return items; };
    const edit = (button = loaded()) => { button.props.onKeyDown(event(button, 'F2')); render(); return input(); };
    const blur = async (draft: string) => { input().props.onBlur({ currentTarget: { value: draft } }); await settle(); render(); };
    render();
    find(node => node.props.className === 'sidebar-shelf-toggle').props.onClick(); render();
    await run({ props, state, source, render, find, loaded, archived, input, event, menu, edit, blur, nodes: () => nodes,
      renames, updates, loads, storage, items: () => items, opened: () => opened, refreshed: () => refreshed, chosen: () => chosen, fail: () => { failure = true; } });
  } finally {
    await vite.close(); delete host.__campaignMenu;
    if (storageBefore) Object.defineProperty(globalThis, 'localStorage', storageBefore); else Reflect.deleteProperty(globalThis, 'localStorage');
  }
}

test('loaded and archive rows offer their own actions through right-click and keyboard menus', async () => fixture(async view => {
  assert.deepEqual(view.menu().map((item: MenuItem) => item.label), ['Rename', 'Change flag', 'Unload']);
  assert.deepEqual(view.menu(view.archived()).map((item: MenuItem) => item.label), ['Rename']);
  for (const [key, shift] of [['ContextMenu', false], ['F10', true]]) {
    view.loaded().props.onKeyDown(view.event(view.loaded(), key, shift));
    assert.deepEqual(view.items().map((item: MenuItem) => item.label), ['Rename', 'Change flag', 'Unload']);
  }
  view.menu().find((item: MenuItem) => item.label === 'Unload').run(); assert.equal(view.chosen(), 1);
  view.menu().find((item: MenuItem) => item.label === 'Rename').run(); view.render();
  assert.equal(view.input().props.defaultValue, 'Loaded');
  view.input().props.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() {} }); view.render();
  delete view.props.client.renameRecording; view.render();
  assert.deepEqual(view.menu().map((item: MenuItem) => item.label), ['Change flag', 'Unload']);
  view.archived().props.onKeyDown(view.event(view.archived(), 'F2')); view.render();
  assert.ok(!view.nodes().some((node: Element) => node.props['aria-label'] === 'Recording name'));
}));

test('inline rename renames the file once on Enter or blur, and what is kept by its path follows', async () => fixture(async view => {
  const campaigns = structuredClone(view.props.campaigns), source = structuredClone(view.source);
  const editor = view.edit(); assert.equal(editor.props.defaultValue, 'Loaded');
  editor.props.onKeyDown({ key: 'Enter', preventDefault() {}, currentTarget: { blur: () => editor.props.onBlur({ currentTarget: { value: '  Barbarossa  ' } }) } });
  editor.props.onBlur({ currentTarget: { value: 'Barbarossa' } });
  await settle(); view.render();
  assert.deepEqual(view.renames, [['C:/recordings/Loaded.battleplan', 'Barbarossa']]);
  assert.deepEqual(view.updates, [['C:/recordings/Loaded.battleplan', 'C:/recordings/Barbarossa.battleplan']]);
  assert.equal(view.refreshed(), 1, 'the library lists the file by its new name');
  assert.deepEqual(view.props.campaigns, campaigns); assert.deepEqual(view.source, source);
  assert.deepEqual([...view.storage.keys()], ['hoi4-replay.sidebar.archive-open'], 'no name is kept apart from the file');
  const cancelled = view.edit(); cancelled.props.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() {} });
  cancelled.props.onBlur({ currentTarget: { value: 'Should not commit' } }); await settle(); view.render();
  for (const draft of ['  ', 'Loaded']) { view.edit(); await view.blur(draft); }
  assert.equal(view.renames.length, 1);
  view.fail(); view.edit(); await view.blur('Blocked');
  assert.equal(view.find((node: Element) => node.props.role === 'alert').props.children, 'This recording is in use.');
  assert.equal(view.loaded().props.children[1].props.children, 'Loaded', 'a refused rename keeps the file name');
  assert.equal(view.updates.length, 1);
}));

test('loaded and archived recordings are named and found by their file names', async () => fixture(async view => {
  assert.equal(view.archived().props.children[1].props.children, 'Archive');
  assert.equal(view.archived().props['aria-label'], 'Load Archive');
  assert.equal(view.archived().props['data-tooltip'], 'Archive.battleplan');
  assert.equal(view.loaded().props['data-tooltip'], 'Loaded.battleplan · Shown');
  const search = (query: string) => { view.find((node: Element) => node.props['aria-label'] === 'Search recordings').props.onChange({ currentTarget: { value: query } }); view.render(); };
  for (const query of ['loaded', 'LOADED.battleplan']) { search(query); assert.equal(view.loaded().props.children[1].props.children, 'Loaded'); }
  for (const query of ['archive', 'ARCHIVE.battleplan']) { search(query); assert.equal(view.archived().props.children[1].props.children, 'Archive'); }
  search('archive'); view.archived().props.onClick(); view.render();
  assert.deepEqual(view.loads, [['C:/recordings/Archive.battleplan', 'Archive.battleplan', 'new']], 'layout labels keep the file name');
}));

test('campaign flag picker uses recorded countries, overrides the shown flag and only offers clear when set', async () => fixture(async view => {
  assert.equal(view.find((node: Element) => 'tag' in node.props && 'source' in node.props).props.tag, 'shown-country');
  view.menu().find((item: MenuItem) => item.label === 'Change flag').run(); view.render();
  assert.equal(view.opened(), 1);
  const picker = view.find((node: Element) => 'allowedTags' in node.props);
  assert.deepEqual(picker.props.allowedTags, ['shown-country', 'mod-country-long-tag']);
  assert.equal(picker.props.countries, view.source.metadata.countries);
  picker.props.onChange('mod-country-long-tag'); view.render();
  assert.equal(view.find((node: Element) => 'tag' in node.props && 'source' in node.props).props.tag, 'mod-country-long-tag');
  assert.deepEqual(view.state.settings.campaigns.ab.factions, { 3: { colour: '#123456' } });
  view.menu().find((item: MenuItem) => item.label === "Use the shown country's flag").run(); view.render();
  assert.equal(view.find((node: Element) => 'tag' in node.props && 'source' in node.props).props.tag, 'shown-country');
  assert.ok(!view.menu().some((item: MenuItem) => item.label === "Use the shown country's flag"));
}));

test('a manually picked recording follows its file when renamed', async () => fixture(async view => {
  view.find((node: Element) => node.props['aria-label'] === 'Open a recording').props.onClick(); await settle(); view.render();
  const picked = view.find((node: Element) => node.props['data-tooltip'] === 'Picked.battleplan');
  view.edit(picked); await view.blur('Renamed pick');
  const renamed = view.find((node: Element) => node.props['data-tooltip'] === 'Renamed pick.battleplan');
  assert.equal(renamed.props.children[1].props.children, 'Renamed pick');
  renamed.props.onClick(); view.render();
  assert.deepEqual(view.loads, [['C:/elsewhere/Picked.battleplan', 'Picked.battleplan', 'new'],
    ['C:/elsewhere/Renamed pick.battleplan', 'Renamed pick.battleplan', 'new']]);
  assert.deepEqual(view.updates, [['C:/elsewhere/Picked.battleplan', 'C:/elsewhere/Renamed pick.battleplan']]);
}));

test('a picked recording follows a rename the dashboard followed, and only that', async () => fixture(async view => {
  view.find((node: Element) => node.props['aria-label'] === 'Open a recording').props.onClick(); await settle(); view.render();
  const picked = 'C:/elsewhere/Picked.battleplan', saved = 'C:/elsewhere/Germany 1936–1937.battleplan';
  view.props.campaigns = [{ ...view.props.campaigns[0], path: 'C:/elsewhere/Other.battleplan' }];
  view.render();
  assert.ok(view.nodes().some((node: Element) => node.props['data-tooltip'] === 'Picked.battleplan'));
  view.props.relocations = [[picked, saved, 'picked recording']];
  view.render(); view.render();
  assert.ok(!view.nodes().some((node: Element) => node.props['data-tooltip'] === 'Picked.battleplan'));
  view.find((node: Element) => node.props['data-tooltip'] === 'Germany 1936–1937.battleplan').props.onClick(); view.render();
  assert.deepEqual(view.loads.at(-1), [saved, 'Germany 1936–1937.battleplan', 'new']);
}));
