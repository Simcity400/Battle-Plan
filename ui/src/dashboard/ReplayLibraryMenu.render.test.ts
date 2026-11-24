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
  const windowBefore = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { addEventListener() {}, removeEventListener() {} } });
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
        return { path: path.replace(/[^/]+$/, `${name}.battleplan`), warning: null };
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
    const archived = () => find(node => node.props['aria-label'] === 'Load Archive');
    const input = () => find(node => node.props['aria-label'] === 'Campaign name');
    const event = (_button: Element, key?: string, shiftKey = false) => ({ key, shiftKey, clientX: 10, clientY: 20, preventDefault() {}, stopPropagation() {},
      currentTarget: { getBoundingClientRect: () => ({ left: 10, bottom: 20 }) } });
    const menu = (button = loaded()) => { button.props.onContextMenu(event(button)); return items; };
    const edit = (button = loaded()) => { button.props.onKeyDown(event(button, 'F2')); render(); return input(); };
    const blur = async (draft: string) => { input().props.onBlur({ currentTarget: { value: draft } }); await settle(); render(); };
    render();
    await run({ props, state, source, render, find, loaded, archived, input, event, menu, edit, blur, nodes: () => nodes,
      renames, updates, loads, storage, items: () => items, opened: () => opened, refreshed: () => refreshed, chosen: () => chosen, fail: () => { failure = true; } });
  } finally {
    await vite.close(); delete host.__campaignMenu;
    if (storageBefore) Object.defineProperty(globalThis, 'localStorage', storageBefore); else Reflect.deleteProperty(globalThis, 'localStorage');
    if (windowBefore) Object.defineProperty(globalThis, 'window', windowBefore); else Reflect.deleteProperty(globalThis, 'window');
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
  assert.ok(!view.nodes().some((node: Element) => node.props['aria-label'] === 'Campaign name'));
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
  assert.deepEqual([...view.storage.keys()], [], 'no name is kept apart from the file');
  const cancelled = view.edit(); cancelled.props.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() {} });
  cancelled.props.onBlur({ currentTarget: { value: 'Should not commit' } }); await settle(); view.render();
  for (const draft of ['  ', 'Loaded']) { view.edit(); await view.blur(draft); }
  assert.equal(view.renames.length, 1);
  view.fail(); view.edit(); await view.blur('Blocked');
  assert.equal(view.find((node: Element) => node.props.className === 'sidebar-failure').props.message, 'This recording is in use.');
  assert.equal(view.loaded().props.children[1].props.children, 'Loaded', 'a refused rename keeps the file name');
  assert.equal(view.updates.length, 1);
}));

test('Enter or Esc in an inline rename gives focus back to its row; leaving it by a click does not', async () => fixture(async view => {
  const focused: string[] = [];
  const row = (name: string) => ({ focus: () => { focused.push(name); }, ownerDocument: { activeElement: null, body: null } });
  view.edit().props.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() {} }); view.render();
  view.loaded().props.ref(row('loaded'));
  view.loaded().props.ref(row('loaded again'));
  assert.deepEqual(focused, ['loaded'], 'Esc focuses the row once');
  view.edit(); await view.blur('Loaded');
  view.loaded().props.ref(row('after a click'));
  assert.deepEqual(focused, ['loaded'], 'a rename left by a click leaves focus where the click put it');
  const editor = view.edit(view.archived());
  editor.props.onKeyDown({ key: 'Enter', preventDefault() {}, currentTarget: { blur: () => editor.props.onBlur({ currentTarget: { value: 'Archive' } }) } });
  view.render(); view.archived().props.ref(row('archive'));
  assert.deepEqual(focused, ['loaded', 'archive'], 'Enter focuses the library row');
}));

test('loaded and archived recordings are named and found by their file names', async () => fixture(async view => {
  assert.equal(view.archived().props.children[1].props.children, 'Archive');
  assert.equal(view.archived().props['aria-label'], 'Load Archive');
  assert.equal(view.archived().props['data-tooltip'], undefined, 'a tooltip never repeats the visible name');
  assert.equal(view.loaded().props['data-tooltip'], 'Shown', 'tooltips name no file extension');
  const field = view.find((node: Element) => node.props.type === 'search');
  assert.equal(field.props['aria-label'], 'Find a campaign');
  assert.equal(field.props.placeholder, 'Find a campaign');
  view.props.library = { ...view.props.library, files: [...view.props.library.files, { path: 'C:/recordings/Old runs/Archive.battleplan', name: 'Archive', folder: 'Old runs' }] }; view.render();
  const twin = view.find((node: Element) => node.props['aria-label'] === 'Load Archive in Old runs');
  const folderLine = twin.props.children[1].props.children[1];
  assert.deepEqual([twin.props['data-tooltip-overflow'], folderLine.props.className, folderLine.props.children.props.children], ['Archive in Old runs', 'sidebar-row-folder', 'Old runs'],
    'a recording that shares its name shows the folder that tells it apart on a line of its own');
  assert.equal(view.find((node: Element) => node.props['aria-label'] === 'Load Archive in recordings').props.children[1].props.children[0].props.children, 'Archive');
  view.props.library = { ...view.props.library, files: view.props.library.files.slice(0, 1) }; view.render();
  const search = (query: string) => { view.find((node: Element) => node.props.type === 'search').props.onChange({ currentTarget: { value: query } }); view.render(); };
  for (const query of ['loaded', 'LOADED.battleplan']) { search(query); assert.equal(view.loaded().props.children[1].props.children, 'Loaded'); }
  for (const query of ['archive', 'ARCHIVE.battleplan']) { search(query); assert.equal(view.archived().props.children[1].props.children, 'Archive'); }
  search('nothing like it');
  assert.equal(view.find((node: Element) => node.props.className === 'sidebar-search-empty').props.children, 'No matching campaigns.');
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
  view.menu().find((item: MenuItem) => item.label === 'Use the shown country’s flag').run(); view.render();
  assert.equal(view.find((node: Element) => 'tag' in node.props && 'source' in node.props).props.tag, 'shown-country');
  assert.ok(!view.menu().some((item: MenuItem) => item.label === 'Use the shown country’s flag'));
}));

test('a manually picked recording follows its file when renamed', async () => fixture(async view => {
  view.find((node: Element) => node.props['aria-label'] === 'Open a recording').props.onClick(); await settle(); view.render();
  const picked = view.find((node: Element) => node.props['aria-label'] === 'Load Picked');
  view.edit(picked); await view.blur('Renamed pick');
  const renamed = view.find((node: Element) => node.props['aria-label'] === 'Load Renamed pick');
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
  assert.ok(view.nodes().some((node: Element) => node.props['aria-label'] === 'Load Picked'));
  view.props.relocations = [[picked, saved, 'picked recording']];
  view.render(); view.render();
  assert.ok(!view.nodes().some((node: Element) => node.props['aria-label'] === 'Load Picked'));
  view.find((node: Element) => node.props['aria-label'] === 'Load Germany 1936–1937').props.onClick(); view.render();
  assert.deepEqual(view.loads.at(-1), [saved, 'Germany 1936–1937.battleplan', 'new']);
}));

test('a recording opened from the library keeps its row while it opens, and a failed load stays on that row with Try again', async () => fixture(async view => {
  const archive = 'C:/recordings/Archive.battleplan';
  const before = view.props.campaigns;
  view.archived().props.onClick();
  view.props.campaigns = [...before, { id: 'second', path: archive, label: 'Archive.battleplan', loading: true }]; view.render();
  const opening = view.find((node: Element) => node.props['aria-label'] === 'Opening Archive');
  assert.equal(opening.props['aria-busy'], true);
  assert.ok(!view.nodes().some((node: Element) => node.props['aria-label'] === 'Show Archive'), 'the row does not move into the loaded list while it opens');
  opening.props.onClick();
  assert.equal(view.loads.length, 1, 'a second click while opening does nothing');
  view.props.campaigns = before; view.props.errors = { [archive]: new Error('broken') }; view.render();
  const failed = view.find((node: Element) => node.props['aria-label'] === 'Load Archive, couldn’t load last time');
  assert.equal(failed.props.children[1].props.children[2].props.children, 'Couldn’t load', 'the failure stays visible on the row');
  assert.equal(view.menu(failed)[0].label, 'Try again');
  view.find((node: Element) => node.props['aria-label'] === 'Try loading Archive again').props.onClick();
  assert.deepEqual(view.loads.map((load: any[]) => load[0]), [archive, archive]);
}));


test('a folder that cannot be read says so with Try again, keeping the rows it last read', async () => fixture(async view => {
  let refreshed = 0;
  view.props.library = { ...view.props.library, error: 'Access denied', refresh: async () => { refreshed++; } }; view.render();
  const alert = view.find((node: Element) => node.props.className === 'sidebar-failure');
  assert.equal(alert.props.message, 'Couldn’t read the recordings folder.');
  assert.equal(alert.props.detail, 'Access denied', 'the raw reason is detail only');
  alert.props.onRetry();
  assert.equal(refreshed, 1);
  assert.ok(view.archived(), 'the last-known library rows stay');
  view.props.campaigns = []; view.props.bindings = {}; view.props.library = { ...view.props.library, files: [] }; view.render();
  const empty = elements(view.find((node: Element) => node.props.className === 'sidebar-campaigns-empty').props.children);
  assert.equal(empty.find(node => node.type === 'p'), undefined, 'an unread folder is not described as empty');
}));


test('a recording opened from a search keeps the search and its row until it opens', async () => fixture(async view => {
  const documentBefore = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { activeElement: null } });
  try {
    const field = () => view.find((node: Element) => node.props.type === 'search');
    field().props.onChange({ currentTarget: { value: 'archive' } }); view.render();
    const archive = 'C:/recordings/Archive.battleplan', before = view.props.campaigns;
    view.archived().props.onClick();
    view.props.campaigns = [...before, { id: 'second', path: archive, label: 'Archive.battleplan', loading: true }]; view.render();
    assert.equal(field().props.value, 'archive', 'the search stays while the recording opens');
    assert.equal(view.find((node: Element) => node.props['aria-label'] === 'Opening Archive').props['aria-busy'], true);
    view.props.campaigns = [...before, { id: 'second', path: archive, label: 'Archive.battleplan' }];
    view.props.bindings = { ...view.props.bindings, second: { source: { ...view.source, id: 'lease-2', path: archive } } };
    view.render(); view.render();
    assert.equal(field().props.value, '', 'the search clears once the recording is open');
  } finally {
    if (documentBefore) Object.defineProperty(globalThis, 'document', documentBefore); else Reflect.deleteProperty(globalThis, 'document');
  }
}));

test('loaded and library rows match a search by file or folder name, and the opened row stays found', async () => fixture(async view => {
  const documentBefore = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { activeElement: null } });
  try {
    const archive = 'C:/recordings/Archive.battleplan', before = view.props.campaigns;
    view.props.library = { ...view.props.library, files: [{ path: archive, name: 'Archive', folder: 'Barbarossa' }, { path: view.source.path, name: 'Loaded', folder: 'Barbarossa' }] };
    const search = (query: string) => { view.find((node: Element) => node.props.type === 'search').props.onChange({ currentTarget: { value: query } }); view.render(); };
    search('barbarossa');
    assert.equal(view.loaded().props.children[1].props.children, 'Loaded', 'a loaded campaign matches its folder as a library row does');
    view.archived().props.onClick();
    view.props.campaigns = [...before, { id: 'second', path: archive, label: 'Archive.battleplan', loading: true }]; view.render();
    view.props.campaigns = [...before, { id: 'second', path: archive, label: 'Archive.battleplan' }];
    view.props.bindings = { ...view.props.bindings, second: { source: { ...view.source, id: 'lease-2', path: archive } } };
    view.render(); view.render();
    assert.ok(view.nodes().some((node: Element) => node.props['aria-label'] === 'Show Archive'), 'the campaign opened from a folder search stays listed');
    assert.ok(!view.nodes().some((node: Element) => node.props.className === 'sidebar-search-empty'));
  } finally {
    if (documentBefore) Object.defineProperty(globalThis, 'document', documentBefore); else Reflect.deleteProperty(globalThis, 'document');
  }
}));

test('a search over an unread folder never claims nothing matches', async () => fixture(async view => {
  view.props.library = { ...view.props.library, error: 'Access denied' }; view.render();
  view.find((node: Element) => node.props.type === 'search').props.onChange({ currentTarget: { value: 'zzz' } }); view.render();
  assert.ok(!view.nodes().some((node: Element) => node.props.className === 'sidebar-search-empty'));
}));

test('a sole search match is marked, and Enter in the search opens or shows it', async () => fixture(async view => {
  const shown: string[] = [];
  view.props.onDefault = (id: string) => { shown.push(id); };
  const field = () => view.find((node: Element) => node.props.type === 'search');
  const search = (query: string) => { field().props.onChange({ currentTarget: { value: query } }); view.render(); };
  const enter = () => { let prevented = false; field().props.onKeyDown({ key: 'Enter', preventDefault() { prevented = true; }, stopPropagation() {} }); view.render(); return prevented; };
  search('battleplan');
  assert.equal(view.archived().props['data-match'], undefined, 'two matches mark neither');
  assert.equal(enter(), false); assert.deepEqual(view.loads, []); assert.deepEqual(shown, []);
  search('arch');
  assert.equal(view.archived().props['data-match'], true);
  assert.equal(enter(), true);
  assert.deepEqual(view.loads.map((load: any[]) => load[0]), ['C:/recordings/Archive.battleplan'], 'Enter opens the sole library match');
  search('load');
  assert.equal(view.loaded().props['data-match'], true);
  enter();
  assert.deepEqual(shown, ['primary'], 'Enter shows the sole loaded match');
  assert.equal(field().props.value, '', 'showing a loaded campaign clears the search');
}));

test('a sidebar failure names its action, keeps the raw reason as detail, retries the same action and clears once it succeeds', async () => fixture(async view => {
  view.fail(); view.edit(); await view.blur('Blocked');
  const line = () => view.nodes().find((node: Element) => node.props.className === 'sidebar-failure');
  assert.equal(line().props.message, 'This recording is in use.', 'a product sentence is shown as it is');
  view.props.client.renameRecording = async (path: string, name: string) => {
    view.renames.push([path, name]);
    if (name === 'Blocked again') throw new Error('C:\recordings\Loaded.battleplan: Access is denied. (os error 5)');
    return { path: path.replace(/[^/]+$/, `${name}.battleplan`), warning: null };
  };
  view.edit(); await view.blur('Blocked again');
  assert.equal(line().props.message, 'Couldn’t rename the recording.');
  assert.equal(line().props.detail, 'C:\recordings\Loaded.battleplan: Access is denied. (os error 5)');
  view.props.client.renameRecording = async (path: string, name: string) => { view.renames.push([path, name]); return { path: path.replace(/[^/]+$/, `${name}.battleplan`), warning: null }; };
  line().props.onRetry(); await settle(); view.render();
  assert.deepEqual(view.renames.at(-1), ['C:/recordings/Loaded.battleplan', 'Blocked again'], 'Try again repeats the rename');
  assert.equal(line(), undefined, 'the line clears once the rename succeeds');
  view.props.client.renameRecording = async (path: string, name: string) => { view.renames.push([path, name]); return { path: path.replace(/[^/]+$/, `${name}.battleplan`), warning: 'the file was replaced, but Windows didn’t confirm it reached the disk' }; };
  view.edit(); await view.blur('Unconfirmed');
  assert.deepEqual([line().props.message, line().props.onRetry, typeof line().props.onDismiss], ['Renamed, but Windows didn’t confirm the name was saved.', undefined, 'function'],
    'a rename Windows applied but didn’t confirm is shown as done, with its warning and nothing to repeat');
  line().props.onDismiss(); view.render();
  assert.equal(line(), undefined);
  view.props.client.chooseRecording = async () => { throw new Error('dialog closed unexpectedly'); };
  view.find((node: Element) => node.props['aria-label'] === 'Open a recording').props.onClick(); await settle(); view.render();
  assert.equal(line().props.message, 'Couldn’t open a recording.');
  view.props.client.chooseRecording = async () => null;
  line().props.onRetry(); await settle(); view.render();
  assert.equal(line(), undefined, 'Try again repeats the pick and clears the line');
}));
