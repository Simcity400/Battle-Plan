import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

test('renaming an open recording retains the top-bar flag and list flag requests', async () => {
  const slots: any[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  let flags = 0;
  let lists = 0;
  let ownerSource: unknown;
  const fixture = {
    useState(initial: any) { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }]; },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useId: () => 'countries',
    useMemo(factory: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (!before || !deps.every((value, index) => Object.is(value, before.deps[index]))) slots[index] = { deps, value: factory() };
      return slots[index].value; },
    useEffect(effect: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (!before || !deps.every((value, index) => Object.is(value, before[index]))) { slots[index] = deps; effects.push(effect); } },
    useLayoutEffect() {},
    async product(_client: unknown, request: { product: string }, _signal: unknown, _validate: unknown, source: unknown) {
      if (request.product === 'land-owners') { ownerSource = source; return { countries: [], warnings: [] }; }
      flags++; return { flags: [] };
    },
    async listFlags() { lists++; },
    reads: new Map<string, unknown>(),
  };
  const host = globalThis as typeof globalThis & { __pickerRename?: typeof fixture };
  host.__pickerRename = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-rename-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/components/CountryPicker.tsx')) return source.replace("import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';",
          'const { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } = globalThis.__pickerRename;');
        if (file.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: true, trigger: { current: null } }); export const Popover = props => props.children;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const forgetSettledProducts = () => {}; export const loadProduct = (...args) => globalThis.__pickerRename.product(...args); export const loadTransientProduct = loadProduct;';
        if (file.endsWith('/dashboard/flagCache.ts')) return 'export class FlagStore { static key() { return "flag"; } } export const flagStore = () => ({ get() {} }); export const loadListFlags = () => globalThis.__pickerRename.listFlags();';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (file.endsWith('/dashboard/countryNamesAt.ts')) return 'const names = new Map(); export const useCountryNamesAt = () => names; export const useCountryNamesRead = () => globalThis.__pickerRename.reads; export const nameKey = () => "";';
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx');
    const countries = [{ id: '1', tag: 'mod-country', name: 'Country', observedBranchId: '1', observedHour: 12 }];
    const props: any = { client: {}, countries, selectedTag: 'mod-country', playerCountryId: '1', onChange() {}, allowedTags: ['mod-country'],
      source: { id: 'lease-1', revision: 'revision-1', path: 'old.battleplan', metadata: { countries, branches: [{ id: '1', firstHour: 12, lastHour: 24 }] } }, flagAt: { branchId: '1', hour: 12 } };
    let output: unknown;
    const render = async () => { cursor = 0; effects = []; output = Picker(props); for (const effect of effects) effect(); await new Promise(resolve => setImmediate(resolve)); };
    const texts = (node: any): string[] => node == null || typeof node === 'boolean' ? [] : typeof node === 'string' || typeof node === 'number' ? [String(node)]
      : Array.isArray(node) ? node.flatMap(texts) : texts(node.props?.children);
    await render(); await render();
    const beforeFlags = flags, beforeLists = lists;
    assert.ok(beforeFlags > 0); assert.ok(beforeLists > 0);
    props.source = { ...props.source, path: 'new.battleplan', metadata: { ...props.source.metadata, path: 'new.battleplan', name: 'new.battleplan' } };
    await render();
    assert.equal(flags, beforeFlags, 'the same source revision and shown hour need no new top-bar flag read');
    assert.equal(lists, beforeLists, 'renaming does not repeat list flag reads');
    props.allowedTags = undefined;
    await render();
    assert.equal(ownerSource, props.source, 'campaign-wide owner discovery uses the prepared polling source');
    // The playback top bar names its country at the day its flag shows, not as first observed.
    fixture.reads.set('', 'Dated name');
    await render(); await render();
    assert.ok(texts(output).includes('Dated name'), 'the top bar names the country at the playback day');
    fixture.reads.set('', null);
    await render(); await render();
    assert.ok(texts(output).includes('Country name unavailable'), 'a day without a recorded name says so');
  } finally { await vite.close(); delete host.__pickerRename; }
});

test('a top-bar flag read finishes across playback days, but a branch switch cancels it', async () => {
  const slots: any[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  const reads: { branchId: string; hour: number; signal: AbortSignal; answer: (reply: unknown) => void }[] = [];
  const fixture = {
    useState(initial: any) { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }]; },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useId: () => 'countries',
    useMemo(factory: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (!before || !deps.every((value, index) => Object.is(value, before.deps[index]))) slots[index] = { deps, value: factory() };
      return slots[index].value; },
    // Effects whose inputs changed clean up first, as React's do.
    useEffect(effect: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (!before || !deps.every((value, index) => Object.is(value, before.deps[index]))) {
        const slot: { deps: any[]; cleanup?: () => void } = { deps };
        before?.cleanup?.();
        slots[index] = slot;
        effects.push(() => { slot.cleanup = effect(); });
      } },
    useLayoutEffect() {},
    product(_client: unknown, request: { params: { branchId: string; hour: number } }, signal: AbortSignal) {
      return new Promise(answer => reads.push({ branchId: request.params.branchId, hour: request.params.hour, signal, answer }));
    },
    reads: new Map<string, unknown>(),
  };
  const host = globalThis as typeof globalThis & { __pickerFlags?: typeof fixture };
  host.__pickerFlags = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-flags-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/components/CountryPicker.tsx')) return source.replace("import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';",
          'const { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } = globalThis.__pickerFlags;');
        if (file.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: false, trigger: { current: null } }); export const Popover = props => props.children;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const forgetSettledProducts = () => {}; export const loadProduct = (...args) => globalThis.__pickerFlags.product(...args); export const loadTransientProduct = loadProduct;';
        if (file.endsWith('/dashboard/flagCache.ts')) return 'export class FlagStore { static key() { return "flag"; } } export const flagStore = () => ({ get() {} }); export const loadListFlags = async () => {};';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (file.endsWith('/dashboard/countryNamesAt.ts')) return 'const names = new Map(); export const useCountryNamesAt = () => names; export const useCountryNamesRead = () => globalThis.__pickerFlags.reads; export const nameKey = () => "";';
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx');
    const countries = [{ id: '1', tag: 'mod-country', name: 'Country', observedBranchId: '1', observedHour: 12 }];
    const branches = [{ id: '1', firstHour: 12, lastHour: 24 }, { id: '2', firstHour: 12, lastHour: 40 }];
    const props: any = { client: {}, countries, selectedTag: 'mod-country', playerCountryId: '1', onChange() {}, allowedTags: ['mod-country'],
      source: { id: 'lease-1', revision: 'revision-1', path: 'one.battleplan', metadata: { countries, branches } }, flagAt: { branchId: '1', hour: 12 } };
    let output: unknown;
    const settle = async () => {
      for (let pass = 0; pass < 3; pass++) {
        cursor = 0; effects = []; output = Picker(props);
        for (const effect of effects) effect();
        await new Promise(resolve => setImmediate(resolve));
      }
    };
    const images = (node: any): unknown[] => node == null || typeof node !== 'object' ? []
      : Array.isArray(node) ? node.flatMap(images) : [...(node.props?.image ? [node.props.image] : []), ...images(node.props?.children)];
    const flag = (name: string) => ({ flags: [{ tag: 'mod-country', name }] });
    await settle();
    assert.deepEqual(reads.map(read => [read.branchId, read.hour]), [['1', 12]]);
    // A newer day waits for the read in flight rather than cancelling it.
    props.flagAt = { branchId: '1', hour: 13 };
    await settle();
    assert.equal(reads.length, 1);
    assert.equal(reads[0].signal.aborted, false);
    reads[0].answer(flag('first-day'));
    await settle();
    assert.deepEqual(images(output), [{ tag: 'mod-country', name: 'first-day' }]);
    assert.deepEqual(reads.map(read => [read.branchId, read.hour]), [['1', 12], ['1', 13]]);
    // Another branch is another country history: its read starts at once and no earlier flag shows.
    props.flagAt = { branchId: '2', hour: 20 };
    await settle();
    assert.equal(reads[1].signal.aborted, true);
    assert.deepEqual(reads.slice(2).map(read => [read.branchId, read.hour]), [['2', 20]]);
    reads[1].answer(flag('cancelled'));
    await settle();
    assert.deepEqual(images(output), []);
    reads[2].answer(flag('second-branch'));
    await settle();
    assert.deepEqual(images(output), [{ tag: 'mod-country', name: 'second-branch' }]);
    assert.equal(reads.length, 3);
  } finally { await vite.close(); delete host.__pickerFlags; }
});
