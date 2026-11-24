import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ProductRequest } from '../dashboard/client';
import type { CountryFactsReply, CountryFlagImage, CountryFlagParams, CountryFlagReply } from '../generated/dashboard';

function pickerHooks() {
  const slots: any[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  const same = (a: any[], b: any[]) => a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
  const useMemo = (factory: () => any, deps: any[]) => {
    const index = cursor++;
    const before = slots[index];
    if (!before || !same(deps, before.deps)) slots[index] = { deps, value: factory() };
    return slots[index].value;
  };
  return {
    useState(initial: any) { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }]; },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useId: () => 'countries',
    useMemo,
    useCallback(fn: any, deps: any[]) { return useMemo(() => fn, deps); },
    useSyncExternalStore(subscribe: (listener: () => void) => () => void, getSnapshot: () => any) {
      const index = cursor++;
      const before = slots[index];
      if (!before || before.subscribe !== subscribe) {
        before?.cleanup?.();
        const slot = { subscribe, cleanup: subscribe(() => {}) };
        slots[index] = slot;
      }
      return getSnapshot();
    },
    useEffect(effect: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (!before || !same(deps, before.deps)) {
        const slot: { deps: any[]; cleanup?: () => void } = { deps };
        slots[index] = slot;
        effects.push(() => { before?.cleanup?.(); slot.cleanup = effect(); });
      } },
    useLayoutEffect() {},
    render(draw: () => unknown) { cursor = 0; effects = []; return draw(); },
    commit() { for (const effect of effects.splice(0)) effect(); },
    unmount() { for (const slot of slots) slot?.cleanup?.(); },
  };
}

const timeline = (branchId: string, key: string, name = 'Country'): CountryFactsReply => ({ branchId, known: [[12, 40]], names: [name], reasons: [],
  flags: [{ keys: [key, key, key], reasons: [null, null, null] }],
  countries: [{ tag: 'mod-country', runs: [[12, 0, 0x336699, -1, 0]] }], warnings: [] });
const drawing = ({ flags, size }: CountryFlagParams): CountryFlagReply => ({ size, unavailable: [], warnings: [],
  flags: flags.map((choice, flag) => ({ flag, asset: `gfx/flags/${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`,
    width: 82, height: 52, png: `http://artwork.localhost/${choice.keys[0]}-${size}.png` })) });
const images = (node: any): CountryFlagImage[] => node == null || typeof node !== 'object' ? []
  : Array.isArray(node) ? node.flatMap(images) : [...(node.props?.image ? [node.props.image] : []), ...images(node.props?.children)];
const texts = (node: any): string[] => node == null || typeof node === 'boolean' ? [] : typeof node === 'string' || typeof node === 'number' ? [String(node)]
  : Array.isArray(node) ? node.flatMap(texts) : texts(node.props?.children);

test('renaming an open recording retains the top-bar and list flags without re-reading', async () => {
  const requests: ProductRequest[] = [];
  let ownerSource: unknown;
  const fixture = {
    ...pickerHooks(),
    async product(_client: unknown, request: ProductRequest, _signal: unknown, _validate: unknown, source: unknown) {
      requests.push(request);
      if (request.product === 'land-owners') { ownerSource = source; return { countries: [{ id: '1', tag: 'mod-country', name: 'Catalogue Name', observedBranchId: '1', observedHour: 12 }], warnings: [] }; }
      if (request.product === 'country-facts') return timeline((request.params as { branchId: string }).branchId, 'mod-country', 'Dated Name');
      if (request.product === 'country-styles') return { styles: [] };
      assert.equal(request.product, 'country-flags');
      return drawing(request.params as CountryFlagParams);
    },
  };
  const host = globalThis as typeof globalThis & { __pickerRename?: typeof fixture };
  host.__pickerRename = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-rename-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/components/CountryPicker.tsx') || file.endsWith('/families/identity/react.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__pickerRename;');
        if (file.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: true, trigger: { current: null } }); export const Popover = props => props.children;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const loadProduct = (...args) => globalThis.__pickerRename.product(...args); export const loadTransientProduct = (c, r, s, src) => globalThis.__pickerRename.product(c, r, s, undefined, src);';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx');
    const countries = [{ id: '1', tag: 'mod-country', name: 'Catalogue Name', observedBranchId: '1', observedHour: 12 }];
    const props: any = { client: {}, countries, selectedTag: 'mod-country', playerCountryId: '1', onChange() {}, allowedTags: ['mod-country'],
      source: { id: 'lease-1', revision: 'revision-1', path: 'old.battleplan', metadata: { countries, branches: [{ id: '1', firstHour: 12, lastHour: 40 }] } }, flagAt: { branchId: '1', hour: 12 } };
    let output: unknown;
    const settle = async () => {
      for (let pass = 0; pass < 4; pass++) {
        output = fixture.render(() => Picker(props)); fixture.commit();
        await new Promise(resolve => setImmediate(resolve));
      }
    };
    await settle();
    assert.ok(requests.some(r => r.product === 'country-facts'));
    const beforeImages = images(output), beforeReads = requests.length;
    assert.deepEqual(beforeImages.map(image => image.key).sort(), ['mod-country:panel', 'mod-country:shield']);
    props.source = { ...props.source, path: 'new.battleplan', metadata: { ...props.source.metadata, path: 'new.battleplan', name: 'new.battleplan' } };
    output = fixture.render(() => Picker(props));
    assert.deepEqual(images(output), beforeImages, 'renaming retains both flags on the first render');
    fixture.commit(); await settle();
    assert.equal(requests.length, beforeReads, 'renaming reuses the branch timeline and both flag drawings');
    assert.deepEqual(images(output), beforeImages);
    props.allowedTags = undefined;
    await settle();
    assert.equal(ownerSource, props.source, 'campaign-wide owner discovery uses the prepared polling source');
    assert.ok(texts(output).includes('Dated Name'), 'the top bar names the country at the playback day');
    assert.equal(texts(output).filter(text => text === 'Dated Name').length, 2, 'its list names it as the top bar does, never by its catalogue entry');
    assert.ok(!texts(output).includes('Catalogue Name'), 'dated roster never falls back to catalogue name');
  } finally { fixture.unmount(); await vite.close(); delete host.__pickerRename; }
});

test('dated picker never leaks future catalogue name while pending and never pairs new name with old flag on change', async () => {
  const requests: { request: ProductRequest; signal: AbortSignal; answer: (reply: unknown) => void }[] = [];
  const fixture = {
    ...pickerHooks(),
    product(_client: unknown, request: ProductRequest, signal: AbortSignal) {
      if (request.product === 'land-owners') {
        return Promise.resolve({ countries: [{ id: '1', tag: 'GER', name: 'Future Empire (hour 300)', observedBranchId: '1', observedHour: 300 }], warnings: [] });
      }
      return new Promise(answer => requests.push({ request, signal, answer }));
    },
  };
  const host = globalThis as typeof globalThis & { __pickerDated?: typeof fixture };
  host.__pickerDated = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-dated-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/components/CountryPicker.tsx') || file.endsWith('/families/identity/react.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__pickerDated;');
        if (file.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: true, trigger: { current: null } }); export const Popover = props => props.children;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const loadProduct = (...args) => globalThis.__pickerDated.product(...args); export const loadTransientProduct = (c, r, s, src) => globalThis.__pickerDated.product(c, r, s, undefined, src);';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx');
    const countries = [{ id: '1', tag: 'GER', name: 'Future Empire (hour 300)', observedBranchId: '1', observedHour: 300 }];
    const props: any = { client: {}, countries, selectedTag: 'GER', playerCountryId: '1', onChange() {},
      source: { id: 'lease-1', revision: 'revision-1', path: 'one.battleplan', metadata: { countries, branches: [{ id: '1', firstHour: 100, lastHour: 300 }] } },
      flagAt: { branchId: '1', hour: 100 } };
    let output: unknown;
    const settle = async () => {
      for (let pass = 0; pass < 3; pass++) {
        output = fixture.render(() => Picker(props)); fixture.commit();
        await new Promise(resolve => setImmediate(resolve));
      }
    };
    await settle();
    assert.ok(!texts(output).includes('Future Empire (hour 300)'), 'pending dated picker must never show hour-300 catalogue name at hour 100');
    assert.ok(texts(output).includes('Loading name…'), 'pending dated picker shows loading');
    assert.deepEqual(images(output), [], 'pending dated picker shows no flag');

    const factsReq = requests.find(r => r.request.product === 'country-facts')!;
    factsReq.answer({
      branchId: '1',
      known: [[100, 300]],
      names: ['Weimar', 'Republic'],
      reasons: [],
      flags: [
        { keys: ['ger_early', 'ger_early', 'ger_early'], reasons: [null, null, null] },
        { keys: ['ger_late', 'ger_late', 'ger_late'], reasons: [null, null, null] },
      ],
      countries: [{ tag: 'GER', runs: [[100, 0, 0x336699, -1, 0], [150, 1, 0x993333, -1, 1]] }],
      warnings: [],
    } satisfies CountryFactsReply);
    await settle();
    for (const r of requests.filter(r => r.request.product === 'country-flags')) {
      r.answer(drawing(r.request.params as CountryFlagParams));
    }
    await settle();
    assert.ok(texts(output).includes('Weimar'));
    assert.ok(images(output).every(img => img.key.startsWith('ger_early:')));

    props.flagAt = { branchId: '1', hour: 150 };
    output = fixture.render(() => Picker(props));
    assert.ok(texts(output).includes('Republic'), 'name switches immediately to hour 150');
    assert.deepEqual(images(output), [], 'old hour-100 flag is never shown beside the new hour-150 name while the new flag draws');
    fixture.commit(); await settle();
    for (const r of requests.filter(r => r.request.product === 'country-flags').slice(2)) {
      r.answer(drawing(r.request.params as CountryFlagParams));
    }
    await settle();
    assert.ok(images(output).length === 2 && images(output).every(img => img.key.startsWith('ger_late:')), 'both header and list show the new hour-150 flag once drawn');
  } finally { fixture.unmount(); await vite.close(); delete host.__pickerDated; }
});

test('playback days reuse the held flag timeline and a branch switch clears the earlier flag', async () => {
  const requests: { request: ProductRequest; signal: AbortSignal; answer: (reply: unknown) => void }[] = [];
  const fixture = {
    ...pickerHooks(),
    product(_client: unknown, request: ProductRequest, signal: AbortSignal) {
      return new Promise(answer => requests.push({ request, signal, answer }));
    },
  };
  const host = globalThis as typeof globalThis & { __pickerFlags?: typeof fixture };
  host.__pickerFlags = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-flags-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/components/CountryPicker.tsx') || file.endsWith('/families/identity/react.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__pickerFlags;');
        if (file.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: false, trigger: { current: null } }); export const Popover = props => props.children;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const loadProduct = (...args) => globalThis.__pickerFlags.product(...args); export const loadTransientProduct = (c, r, s, src) => globalThis.__pickerFlags.product(c, r, s, undefined, src);';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx');
    const countries = [{ id: '1', tag: 'mod-country', name: 'Country', observedBranchId: '1', observedHour: 12 }];
    const branches = [{ id: '1', firstHour: 12, lastHour: 40 }, { id: '2', firstHour: 12, lastHour: 40 }];
    const props: any = { client: {}, countries, selectedTag: 'mod-country', playerCountryId: '1', onChange() {}, allowedTags: ['mod-country'],
      source: { id: 'lease-1', revision: 'revision-1', path: 'one.battleplan', metadata: { countries, branches } }, flagAt: { branchId: '1', hour: 12 } };
    let output: unknown;
    const settle = async () => {
      for (let pass = 0; pass < 3; pass++) {
        output = fixture.render(() => Picker(props)); fixture.commit();
        await new Promise(resolve => setImmediate(resolve));
      }
    };
    await settle();
    const facts1 = requests.find(r => r.request.product === 'country-facts' && (r.request.params as { branchId: string }).branchId === '1')!;
    assert.ok(facts1);
    props.flagAt = { branchId: '1', hour: 13 };
    await settle();
    assert.equal(requests.filter(r => r.request.product === 'country-facts').length, 1, 'a new playback day shares the branch timeline read in flight');
    assert.equal(facts1.signal.aborted, false);
    facts1.answer(timeline('1', 'first-branch'));
    await settle();
    const shield1Req = requests.find(r => r.request.product === 'country-flags' && (r.request.params as CountryFlagParams).size === 'shield')!;
    assert.ok(shield1Req);
    const firstFlag = drawing(shield1Req.request.params as CountryFlagParams);
    shield1Req.answer(firstFlag);
    await settle();
    assert.deepEqual(images(output), firstFlag.flags);
    const countBeforeDay14 = requests.length;
    props.flagAt = { branchId: '1', hour: 14 };
    await settle();
    assert.equal(requests.length, countBeforeDay14, 'a held day with the same choice reads and draws nothing new');
    assert.deepEqual(images(output), firstFlag.flags);
    props.flagAt = { branchId: '2', hour: 20 };
    output = fixture.render(() => Picker(props));
    assert.deepEqual(images(output), [], 'the old branch flag is cleared before effects or replies');
    fixture.commit(); await settle();
    const facts2 = requests.find(r => r.request.product === 'country-facts' && (r.request.params as { branchId: string }).branchId === '2')!;
    assert.ok(facts2);
    assert.deepEqual(images(output), [], 'an unread branch timeline cannot borrow the earlier flag');
    facts2.answer(timeline('2', 'second-branch'));
    await settle();
    const shield2Req = requests.filter(r => r.request.product === 'country-flags' && (r.request.params as CountryFlagParams).size === 'shield').at(-1)!;
    assert.deepEqual(images(output), [], 'the new branch choice waits for its own drawing');
    const secondFlag = drawing(shield2Req.request.params as CountryFlagParams);
    shield2Req.answer(secondFlag);
    await settle();
    assert.deepEqual(images(output), secondFlag.flags);
  } finally { fixture.unmount(); await vite.close(); delete host.__pickerFlags; }
});
