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
  return {
    useState(initial: any) { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }]; },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useId: () => 'countries',
    useMemo(factory: () => any, deps: any[]) { const index = cursor++; const before = slots[index];
      if (!before || !same(deps, before.deps)) slots[index] = { deps, value: factory() };
      return slots[index].value; },
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

const timeline = (branchId: string, key: string): CountryFactsReply => ({ branchId, known: [[12, 40]], names: ['Country'], reasons: [],
  flags: [{ keys: [key, key, key], reasons: [null, null, null] }],
  countries: [{ tag: 'mod-country', runs: [[12, 0, 0x336699, -1, 0]] }], warnings: [] });
const drawing = ({ flags, size }: CountryFlagParams): CountryFlagReply => ({ size, unavailable: [], warnings: [],
  flags: flags.map((choice, flag) => ({ flag, asset: `gfx/flags/${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`,
    width: 82, height: 52, png: `http://artwork.localhost/${choice.keys[0]}-${size}.png` })) });
const images = (node: any): CountryFlagImage[] => node == null || typeof node !== 'object' ? []
  : Array.isArray(node) ? node.flatMap(images) : [...(node.props?.image ? [node.props.image] : []), ...images(node.props?.children)];

test('renaming an open recording retains the top-bar and list flags without re-reading', async () => {
  const requests: ProductRequest[] = [];
  let ownerSource: unknown;
  const fixture = {
    ...pickerHooks(),
    async product(_client: unknown, request: ProductRequest, _signal: unknown, _validate: unknown, source: unknown) {
      requests.push(request);
      if (request.product === 'land-owners') { ownerSource = source; return { countries: [], warnings: [] }; }
      if (request.product === 'country-facts') return timeline((request.params as { branchId: string }).branchId, 'mod-country');
      assert.equal(request.product, 'country-flags');
      return drawing(request.params as CountryFlagParams);
    },
    reads: new Map<string, unknown>(),
  };
  const host = globalThis as typeof globalThis & { __pickerRename?: typeof fixture };
  host.__pickerRename = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-rename-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/components/CountryPicker.tsx') || file.endsWith('/dashboard/countryFlags.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__pickerRename;');
        if (file.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: true, trigger: { current: null } }); export const Popover = props => props.children;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const loadProduct = (...args) => globalThis.__pickerRename.product(...args); export const loadTransientProduct = loadProduct;';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (file.endsWith('/dashboard/countryNamesAt.ts')) return 'const names = new Map(); export const useCountryNamesAt = () => names; export const useCountryNamesRead = () => globalThis.__pickerRename.reads; export const nameKey = () => "";';
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/components/CountryPicker.tsx');
    const countries = [{ id: '1', tag: 'mod-country', name: 'Country', observedBranchId: '1', observedHour: 12 }];
    const props: any = { client: {}, countries, selectedTag: 'mod-country', playerCountryId: '1', onChange() {}, allowedTags: ['mod-country'],
      source: { id: 'lease-1', revision: 'revision-1', path: 'old.battleplan', metadata: { countries, branches: [{ id: '1', firstHour: 12, lastHour: 40 }] } }, flagAt: { branchId: '1', hour: 12 } };
    let output: unknown;
    const settle = async () => {
      for (let pass = 0; pass < 3; pass++) {
        output = fixture.render(() => Picker(props)); fixture.commit();
        await new Promise(resolve => setImmediate(resolve));
      }
    };
    const texts = (node: any): string[] => node == null || typeof node === 'boolean' ? [] : typeof node === 'string' || typeof node === 'number' ? [String(node)]
      : Array.isArray(node) ? node.flatMap(texts) : texts(node.props?.children);
    await settle();
    assert.deepEqual(requests.map(request => request.product), ['country-facts', 'country-flags', 'country-flags']);
    const beforeImages = images(output), beforeReads = requests.length;
    assert.deepEqual(beforeImages.map(image => image.key), ['mod-country:shield', 'mod-country:panel']);
    props.source = { ...props.source, path: 'new.battleplan', metadata: { ...props.source.metadata, path: 'new.battleplan', name: 'new.battleplan' } };
    output = fixture.render(() => Picker(props));
    assert.deepEqual(images(output), beforeImages, 'renaming retains both flags on the first render');
    fixture.commit(); await settle();
    assert.equal(requests.length, beforeReads, 'renaming reuses the branch timeline and both flag drawings');
    assert.deepEqual(images(output), beforeImages);
    props.allowedTags = undefined;
    await settle();
    assert.equal(ownerSource, props.source, 'campaign-wide owner discovery uses the prepared polling source');
    fixture.reads.set('', 'Dated name');
    await settle();
    assert.ok(texts(output).includes('Dated name'), 'the top bar names the country at the playback day');
    fixture.reads.set('', null);
    await settle();
    assert.ok(texts(output).includes('Country name unavailable'), 'a day without a recorded name says so');
  } finally { fixture.unmount(); await vite.close(); delete host.__pickerRename; }
});

test('playback days reuse the held flag timeline and a branch switch clears the earlier flag', async () => {
  const requests: { request: ProductRequest; signal: AbortSignal; answer: (reply: unknown) => void }[] = [];
  const fixture = {
    ...pickerHooks(),
    product(_client: unknown, request: ProductRequest, signal: AbortSignal) {
      return new Promise(answer => requests.push({ request, signal, answer }));
    },
    reads: new Map<string, unknown>(),
  };
  const host = globalThis as typeof globalThis & { __pickerFlags?: typeof fixture };
  host.__pickerFlags = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-flags-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/components/CountryPicker.tsx') || file.endsWith('/dashboard/countryFlags.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__pickerFlags;');
        if (file.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: false, trigger: { current: null } }); export const Popover = props => props.children;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const loadProduct = (...args) => globalThis.__pickerFlags.product(...args); export const loadTransientProduct = loadProduct;';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (file.endsWith('/dashboard/countryNamesAt.ts')) return 'const names = new Map(); export const useCountryNamesAt = () => names; export const useCountryNamesRead = () => globalThis.__pickerFlags.reads; export const nameKey = () => "";';
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
    const asked = () => requests.map(({ request }) => [request.product, request.params]);
    await settle();
    assert.deepEqual(asked(), [['country-facts', { branchId: '1' }]]);
    props.flagAt = { branchId: '1', hour: 13 };
    await settle();
    assert.equal(requests.length, 1, 'a new playback day shares the branch timeline read in flight');
    assert.equal(requests[0].signal.aborted, false);
    requests[0].answer(timeline('1', 'first-branch'));
    await settle();
    const firstDrawing = requests[1].request.params as CountryFlagParams;
    assert.deepEqual(asked(), [['country-facts', { branchId: '1' }], ['country-flags', { flags: timeline('1', 'first-branch').flags, size: 'shield' }]]);
    const firstFlag = drawing(firstDrawing);
    requests[1].answer(firstFlag);
    await settle();
    assert.deepEqual(images(output), firstFlag.flags);
    props.flagAt = { branchId: '1', hour: 14 };
    await settle();
    assert.equal(requests.length, 2, 'a held day with the same choice reads and draws nothing new');
    assert.deepEqual(images(output), firstFlag.flags);
    props.flagAt = { branchId: '2', hour: 20 };
    output = fixture.render(() => Picker(props));
    assert.deepEqual(images(output), [], 'the old branch flag is cleared before effects or replies');
    fixture.commit(); await settle();
    assert.deepEqual(asked().slice(2), [['country-facts', { branchId: '2' }]]);
    assert.deepEqual(images(output), [], 'an unread branch timeline cannot borrow the earlier flag');
    requests[2].answer(timeline('2', 'second-branch'));
    await settle();
    assert.deepEqual(asked().slice(3), [['country-flags', { flags: timeline('2', 'second-branch').flags, size: 'shield' }]]);
    assert.deepEqual(images(output), [], 'the new branch choice waits for its own drawing');
    const secondFlag = drawing(requests[3].request.params as CountryFlagParams);
    requests[3].answer(secondFlag);
    await settle();
    assert.deepEqual(images(output), secondFlag.flags);
    assert.equal(requests.length, 4);
  } finally { fixture.unmount(); await vite.close(); delete host.__pickerFlags; }
});
