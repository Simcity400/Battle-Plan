import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentProps, ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type CountryPicker from './CountryPicker';
import type { CountryPickerRead } from './CountryPicker';
import { nameKey } from '../dashboard/countryFacts.ts';

test('the actual Picker uses per-choice witnesses lazily and masks obsolete names and flags', async () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `actor ${String(index).padStart(4, '0')}`);
  const reads = new Map<string, CountryPickerRead | null>(tags.map((tag, index) => [tag, { tag: `read ${index}`, branchId: '1', hour: 100 + index % 2 }]));
  reads.set(tags[1], null); reads.set(tags[2], { tag: 'foreign', branchId: '2', hour: 100 });
  reads.set(tags[3], { tag: 'outside', branchId: '1', hour: 99 }); reads.delete(tags[4]);
  const state = { hooks: [] as unknown[], cursor: 0, effects: [] as (() => void)[], open: true, content: '1',
    names: new Map([[nameKey(100, 'read 0'), ''], [nameKey(101, 'read 4999'), 'Selected historical name']]),
    wanted: [] as { tag: string; hour: number }[], listReads: [] as { tag: string; branchId: string; hour: number }[][],
    headerReads: [] as { tag: string; hour: number }[],
    rows: [] as { onClick: () => void; 'aria-label'?: string; 'data-tooltip'?: string }[],
    scroll: undefined as ((event: { currentTarget: { scrollTop: number; clientHeight: number } }) => void) | undefined,
    search: undefined as ((event: { currentTarget: { value: string } }) => void) | undefined,
    changed: [] as (string | null)[], rosterLoads: 0, rosterError: null as string | null,
    rosterReply: null as unknown, flagsError: null as string | null, retryAction: undefined as (() => void) | undefined };
  const globals = globalThis as typeof globalThis & { __pickerWitness?: typeof state };
  globals.__pickerWitness = state;
  // Retain the real component's hook cells and effect cleanup between SSR draws.
  const hooks = `const cell = init => { const s = globalThis.__pickerWitness, i = s.cursor++; return s.hooks[i] ??= init(); };
    export const useId = () => cell(() => 'picker-fixture');
    export const useRef = initial => cell(() => ({ current: initial }));
    export const useState = initial => { const saved = cell(() => ({ value: typeof initial === 'function' ? initial() : initial }));
      return [saved.value, next => { saved.value = typeof next === 'function' ? next(saved.value) : next; }]; };
    export const useMemo = (make, deps) => { const saved = cell(() => ({ deps, value: make() }));
      if (saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) { saved.deps = deps; saved.value = make(); }
      return saved.value; };
    export const useLayoutEffect = () => {};
    export const useEffect = (effect, deps) => { const saved = cell(() => ({ deps: null, cleanup: null }));
      if (!saved.deps || saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) {
        saved.deps = deps; globalThis.__pickerWitness.effects.push(() => { saved.cleanup?.(); saved.cleanup = effect(); });
      } };`;
  const replacements: Record<string, string> = {
    '/components/Popover.tsx': `import { Children, createElement, isValidElement } from 'react';
      export const usePopover = () => ({ open: globalThis.__pickerWitness.open, trigger: { current: null }, close() { globalThis.__pickerWitness.open = false; } });
      export const Popover = props => { const s = globalThis.__pickerWitness;
        const walk = nodes => Children.forEach(nodes, node => { if (!isValidElement(node)) return;
          if (node.props.role === 'option') s.rows.push(node.props);
          if (node.props.className === 'country-picker-list') s.scroll = node.props.onScroll;
          if (node.props['aria-label'] === 'Search countries') s.search = node.props.onChange;
          if (node.type === 'button' && node.props.children === 'Retry') s.retryAction = node.props.onClick;
          walk(node.props.children); }); walk(props.children); return createElement('div', null, props.children); };`,
    '/components/CountryFlag.tsx': `import { createElement } from 'react'; export const CountryFlag = ({ image }) => createElement('span', { 'data-flag': image?.tag });`,
    '/components/Icons.tsx': `import { createElement } from 'react'; export const Icon = ({ name }) => createElement('i', { 'data-icon': name });`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => globalThis.__pickerWitness.content;',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
      export const useCountryNamesAt = (_client, _track, wanted) => { const s = globalThis.__pickerWitness; s.wanted = wanted;
        return new Map(wanted.flatMap(({ tag, hour }) => { const key = nameKey(hour, tag); return s.names.has(key) ? [[key, s.names.get(key)]] : []; })); };`,
    '/dashboard/products.ts': `export const forgetSettledProducts = () => {};
      export const loadProduct = (_client, request) => {
        const s = globalThis.__pickerWitness;
        if (request.product === 'country-flags') return Promise.resolve({ flags: [] });
        s.rosterLoads++;
        if (s.rosterError) return Promise.reject(Error(s.rosterError));
        if (!s.rosterReply) throw Error('authoritative choices must not request metadata fallback');
        return Promise.resolve(s.rosterReply);
      };
      export const loadTransientProduct = (_client, request) => { const s = globalThis.__pickerWitness, p = request.params;
        s.headerReads.push({ tag: p.tags[0], hour: p.hour }); return Promise.resolve({ flags: [{ tag: p.tags[0] }] }); };`,
    '/dashboard/flagCache.ts': `export class FlagStore { static key(...args) { return JSON.stringify(args); } }
      export const flagStore = () => ({ get: () => undefined });
      export const loadListFlags = (_client, _source, targets) => { const s = globalThis.__pickerWitness; s.listReads.push(targets);
        return s.flagsError ? Promise.reject(Error(s.flagsError)) : Promise.resolve(); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-witness-fixture', enforce: 'pre',
      resolveId(id) { if (id === 'picker-fixture-hooks') return '\0picker-fixture-hooks'; },
      load(id) { if (id === '\0picker-fixture-hooks') return hooks; },
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/components/CountryPicker.tsx')) return source.replace("from 'react'", "from 'picker-fixture-hooks'");
      },
    }] });
  try {
    const Picker = (await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx')).default as ComponentType<ComponentProps<typeof CountryPicker>>;
    const countries = [{ id: 1, tag: tags[1], name: 'Metadata belongs to another holder', observedBranchId: '1', observedHour: 110 }];
    const props = { countries, allowedTags: tags, selectedTag: tags.at(-1), playerCountryId: null, field: true,
      source: { id: 'synthetic', revision: '1', metadata: { countries, branches: [{ id: '1', firstHour: 100, lastHour: 120 }] } },
      client: {}, flagAt: { branchId: '1', hour: 110 }, choiceReads: reads, onChange: (tag: string | null) => state.changed.push(tag),
    } as unknown as ComponentProps<typeof CountryPicker>;
    const render = () => { state.cursor = 0; state.rows = []; return renderToStaticMarkup(createElement(Picker, props)); };
    const flush = async () => { const effects = state.effects.splice(0); effects.forEach(effect => effect()); await Promise.resolve(); await Promise.resolve(); };
    let html = render(); await flush(); html = render();
    assert.ok(state.rows.length < 30, '5000 choices draw only the viewport');
    assert.ok(state.wanted.length < 30, 'names request the viewport plus the selected trigger');
    assert.ok(state.wanted.some(read => read.tag === 'read 4999' && read.hour === 101));
    assert.ok(state.wanted.every(read => read.tag.startsWith('read ')), 'null, missing, wrong-branch and out-of-range witnesses never fall back');
    assert.ok(state.listReads.flat().every(read => read.tag.startsWith('read ') && read.hour <= 101));
    assert.match(html, /Selected historical name/); assert.doesNotMatch(html, /Metadata belongs/);
    assert.doesNotMatch(html, /no recorded read|Name and flag unavailable|data-icon="alert"/);
    assert.equal(state.rows[1]['aria-label'], undefined, 'missing looks leave the fallback country name visible');
    assert.equal(state.rows[1]['data-tooltip'], undefined);
    assert.match(html, /actor 0002/);
    assert.deepEqual(state.headerReads, [{ tag: 'read 4999', hour: 101 }]);
    state.rows[0].onClick(); assert.deepEqual(state.changed, [tags[0]], 'a mapped alias never replaces the pickable actor');
    state.open = true; props.selectedTag = tags[0]; render(); await flush(); html = render();
    assert.match(html, /<span><\/span>/, 'a proved empty name stays empty'); assert.match(html, /data-flag="read 0"/);
    props.choiceReads = new Map(reads).set(tags[0], null);
    html = render(); assert.doesNotMatch(html, /data-flag="read 0"/); await flush();
    assert.ok(!state.wanted.some(read => read.tag === 'read 0'), 'changed witnesses release old names immediately');
    props.choiceReads = new Map(reads).set(tags[0], { tag: 'successor', branchId: '1', hour: 102 });
    state.names.set(nameKey(102, 'successor'), 'Successor name'); render(); await flush(); html = render();
    assert.match(html, /Successor name/); assert.match(html, /data-flag="successor"/);
    props.client = {} as never; state.names.clear(); html = render();
    assert.doesNotMatch(html, /Successor name|data-flag="successor"/, 'a new client cannot show the old client\'s looks');
    state.names.set(nameKey(102, 'successor'), 'New client name'); await flush(); render(); await flush();
    state.content = '2'; state.names.clear(); html = render(); assert.doesNotMatch(html, /New client name|data-flag="successor"/);
    state.scroll!({ currentTarget: { scrollTop: 36000, clientHeight: 340 } }); render(); await flush();
    assert.ok(state.wanted.length < 30); assert.ok(state.wanted.some(read => read.tag === 'read 1000'));
    state.search!({ currentTarget: { value: 'Searched name' } }); render(); await flush();
    assert.ok(state.wanted.some(read => read.tag === 'read 4000' && read.hour === 100), 'a search asks for names of rows not yet viewed');
    state.names.set(nameKey(100, 'read 4000'), 'Searched name'); render(); await flush(); html = render();
    assert.match(html, /Searched name/, 'the searched name finds its row');
    state.search!({ currentTarget: { value: '' } }); render(); await flush();
    assert.ok(state.wanted.length < 30, 'clearing the search asks only for the rows in view again');
    state.open = false; render(); assert.deepEqual(state.wanted, [{ tag: 'successor', hour: 102 }], 'closing the menu retains only the selected trigger');
    assert.equal(state.rosterLoads, 0, 'authoritative choices never request the metadata roster');
    state.open = true; state.scroll!({ currentTarget: { scrollTop: 0, clientHeight: 340 } });
    props.allowedTags = undefined; props.choiceReads = undefined; props.flagAt = null;
    state.rosterError = 'Synthetic native roster request failed';
    html = render(); assert.match(html, /Loading countries…/);
    await flush(); html = render();
    assert.match(html, /Couldn’t load countries\./);
    assert.match(html, />Retry</);
    assert.doesNotMatch(html, /Synthetic native roster request failed/);
    const failedLoads = state.rosterLoads;
    state.rosterError = null; state.rosterReply = { countries: props.countries, warnings: ['Synthetic ownership diagnostic'] };
    state.flagsError = 'Synthetic decorative flag failure';
    assert.ok(state.retryAction); state.retryAction(); render(); await flush(); html = render(); await flush(); html = render();
    assert.equal(state.rosterLoads, failedLoads + 1, 'Retry actually reloads the countries');
    assert.match(html, /Metadata belongs to another holder/);
    assert.doesNotMatch(html, /Couldn’t load countries|Synthetic ownership diagnostic|Flags unavailable|Some ownership history is unavailable|Synthetic decorative flag failure|class="pill"/);
  } finally { await vite.close(); delete globals.__pickerWitness; }
});

test('the listbox names its active option only while that row is drawn, and is multiselectable for a multi-country field', async () => {
  const state = { hooks: [] as unknown[], cursor: 0, listbox: undefined as Record<string, any> | undefined,
    scroll: undefined as ((event: { currentTarget: { scrollTop: number; clientHeight: number } }) => void) | undefined };
  const globals = globalThis as typeof globalThis & { __pickerList?: typeof state };
  globals.__pickerList = state;
  const hooks = `const cell = init => { const s = globalThis.__pickerList, i = s.cursor++; return s.hooks[i] ??= init(); };
    export const useId = () => cell(() => 'picker-list');
    export const useRef = initial => cell(() => ({ current: initial }));
    export const useState = initial => { const saved = cell(() => ({ value: typeof initial === 'function' ? initial() : initial }));
      return [saved.value, next => { saved.value = typeof next === 'function' ? next(saved.value) : next; }]; };
    export const useMemo = (make, deps) => { const saved = cell(() => ({ deps, value: make() }));
      if (saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) { saved.deps = deps; saved.value = make(); }
      return saved.value; };
    export const useLayoutEffect = () => {};
    export const useEffect = () => {};`;
  const replacements: Record<string, string> = {
    '/components/Popover.tsx': `import { Children, createElement, isValidElement } from 'react';
      export const usePopover = () => ({ open: true, trigger: { current: null }, close() {} });
      export const Popover = props => { const s = globalThis.__pickerList;
        const walk = nodes => Children.forEach(nodes, node => { if (!isValidElement(node)) return;
          if (node.props.role === 'listbox') s.listbox = node.props;
          if (node.props.className === 'country-picker-list') s.scroll = node.props.onScroll;
          walk(node.props.children); }); walk(props.children); return createElement('div', null, props.children); };`,
    '/components/CountryFlag.tsx': `import { createElement } from 'react'; export const CountryFlag = () => createElement('span');`,
    '/components/Icons.tsx': `import { createElement } from 'react'; export const Icon = ({ name }) => createElement('i', { 'data-icon': name });`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map();',
    '/dashboard/products.ts': `export const forgetSettledProducts = () => {}; export const loadProduct = () => new Promise(() => {});
      export const loadTransientProduct = loadProduct;`,
    '/dashboard/flagCache.ts': `export class FlagStore { static key(...args) { return JSON.stringify(args); } }
      export const flagStore = () => ({ get: () => undefined }); export const loadListFlags = () => new Promise(() => {});`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-list-fixture', enforce: 'pre',
      resolveId(id) { if (id === 'picker-list-hooks') return '\0picker-list-hooks'; },
      load(id) { if (id === '\0picker-list-hooks') return hooks; },
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/components/CountryPicker.tsx')) return source.replace("from 'react'", "from 'picker-list-hooks'");
      },
    }] });
  try {
    const Picker = (await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx')).default as ComponentType<ComponentProps<typeof CountryPicker>>;
    const tags = Array.from({ length: 200 }, (_, index) => `T${String(index).padStart(3, '0')}`);
    const countries = tags.map((tag, index) => ({ id: index + 1, tag, name: `Country ${String(index).padStart(3, '0')}` }));
    const props = { countries, allowedTags: tags, selectedTag: null, playerCountryId: null, field: true, chosen: [tags[0], tags[1]], addLabel: 'Add or remove a country',
      source: { id: 'synthetic', revision: '1', metadata: { countries, branches: [{ id: '1', firstHour: 100, lastHour: 120 }] } },
      client: {}, flagAt: null, onChange: () => {},
    } as unknown as ComponentProps<typeof CountryPicker>;
    const render = () => { state.cursor = 0; return renderToStaticMarkup(createElement(Picker, props)); };
    const press = (key: string) => state.listbox!.onKeyDown({ key, preventDefault() {} });
    render();
    assert.equal(state.listbox!['aria-multiselectable'], true, 'chosen countries accumulate');
    assert.equal(state.listbox!['aria-activedescendant'], undefined);
    press('ArrowDown'); let html = render();
    assert.equal(state.listbox!['aria-activedescendant'], 'picker-list-country-0');
    assert.match(html, /id="picker-list-country-0"/);
    state.scroll!({ currentTarget: { scrollTop: 100 * 36, clientHeight: 340 } }); html = render();
    assert.doesNotMatch(html, /id="picker-list-country-0"/, 'the active row scrolled out of the drawn rows');
    assert.equal(state.listbox!['aria-activedescendant'], undefined, 'no active option names a row that is not drawn');
    press('ArrowDown'); html = render();
    assert.equal(state.listbox!['aria-activedescendant'], 'picker-list-country-1', 'moving on brings the active row back into view');
    assert.match(html, /id="picker-list-country-1"/);
    props.chosen = undefined; props.addLabel = undefined; render();
    assert.equal(state.listbox!['aria-multiselectable'], false, 'one country is chosen at a time');
  } finally { await vite.close(); delete globals.__pickerList; }
});
