import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

test('renaming an open recording retains the top-bar flag and list flag requests', async () => {
  const slots: any[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  let flags = 0;
  let lists = 0;
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
    async product() { flags++; return { flags: [] }; },
    async listFlags() { lists++; },
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
        if (file.endsWith('/dashboard/products.ts')) return 'export const forgetSettledProducts = () => {}; export const loadProduct = () => globalThis.__pickerRename.product(); export const loadTransientProduct = loadProduct;';
        if (file.endsWith('/dashboard/flagCache.ts')) return 'export class FlagStore { static key() { return "flag"; } } export const flagStore = () => ({ get() {} }); export const loadListFlags = () => globalThis.__pickerRename.listFlags();';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (file.endsWith('/dashboard/countryNamesAt.ts')) return 'const names = new Map(); export const useCountryNamesAt = () => names; export const nameKey = () => "";';
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx');
    const countries = [{ id: '1', tag: 'mod-country', name: 'Country', observedBranchId: '1', observedHour: 12 }];
    const props: any = { client: {}, countries, selectedTag: 'mod-country', playerCountryId: '1', onChange() {}, allowedTags: ['mod-country'],
      source: { id: 'lease-1', revision: 'revision-1', path: 'old.battleplan', metadata: { countries, branches: [{ id: '1', firstHour: 12, lastHour: 24 }] } }, flagAt: { branchId: '1', hour: 12 } };
    const render = async () => { cursor = 0; effects = []; Picker(props); for (const effect of effects) effect(); await new Promise(resolve => setImmediate(resolve)); };
    await render(); await render();
    const beforeFlags = flags, beforeLists = lists;
    assert.ok(beforeFlags > 0); assert.ok(beforeLists > 0);
    props.source = { ...props.source, path: 'new.battleplan', metadata: { ...props.source.metadata, path: 'new.battleplan', name: 'new.battleplan' } };
    await render();
    assert.equal(flags, beforeFlags, 'the same source revision and shown hour need no new top-bar flag read');
    assert.equal(lists, beforeLists, 'renaming does not repeat list flag reads');
  } finally { await vite.close(); delete host.__pickerRename; }
});
