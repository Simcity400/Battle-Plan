import assert from 'node:assert/strict';
import test, { mock } from 'node:test';
import { createServer } from 'vite';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { ProductRequest } from '../../dashboard/client';
import type { WeatherReply } from './weatherData';

/** Hook state kept across calls, as React keeps it across renders; `render` runs a
 * hook again with the same slots, and effects run as React would after it. */
const FAKE_REACT = `
const slots = []; let cursor = 0;
export const render = hook => { cursor = 0; return hook(); };
export const reset = () => { slots.length = 0; };
const changed = (slot, deps) => !slot || !deps || deps.length !== slot.deps.length || deps.some((value, index) => !Object.is(value, slot.deps[index]));
export function useRef(value) { const index = cursor++; return slots[index] ??= { current: value }; }
export function useState(initial) {
  const index = cursor++;
  if (!(index in slots)) slots[index] = { value: typeof initial === 'function' ? initial() : initial };
  const slot = slots[index];
  return [slot.value, next => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
}
export function useReducer(reducer, initial) {
  const index = cursor++;
  if (!(index in slots)) slots[index] = { value: initial };
  const slot = slots[index];
  return [slot.value, action => { slot.value = reducer(slot.value, action); }];
}
export function useCallback(callback, deps) { const index = cursor++; if (changed(slots[index], deps)) slots[index] = { deps, value: callback }; return slots[index].value; }
export function useMemo(factory, deps) { const index = cursor++; if (changed(slots[index], deps)) slots[index] = { deps, value: factory() }; return slots[index].value; }
export function useEffect(effect, deps) { const index = cursor++; if (changed(slots[index], deps)) { slots[index]?.cleanup?.(); slots[index] = { deps, cleanup: effect() }; } }
`;

interface Load { request: ProductRequest; signal: AbortSignal; resolve: (value: unknown) => void; reject: (error: Error) => void }
interface Fixture { loads: Load[]; products: (ProductRequest | null)[]; product: (request: ProductRequest | null) => { data?: unknown; error?: Error }; content?: number }
const host = globalThis as typeof globalThis & { __weatherHooks?: Fixture };

async function mountHooks() {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, logLevel: 'silent',
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'weather-hooks', enforce: 'pre',
      resolveId: id => id === 'fake-react' ? '\0fake-react' : undefined,
      load: id => id === '\0fake-react' ? FAKE_REACT : undefined,
      transform(code, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/widgets/weather-by-region/useWeather.ts')) return code.replace("from 'react'", "from 'fake-react'");
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => globalThis.__weatherHooks.content ?? 0;';
        if (path.endsWith('/dashboard/products.ts')) return `
          export const useProduct = (_client, request) => { globalThis.__weatherHooks.products.push(request); return { ...globalThis.__weatherHooks.product(request), retry() {} }; };
          export const loadTransientProduct = (_client, request, signal) => new Promise((resolve, reject) =>
            globalThis.__weatherHooks.loads.push({ request, signal, resolve, reject }));
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`;
      } }] });
  const hooks = await vite.ssrLoadModule('/ui/src/widgets/weather-by-region/useWeather.ts') as typeof import('./useWeather.ts');
  const react = await vite.ssrLoadModule('fake-react') as { render: <T>(hook: () => T) => T; reset: () => void };
  return { hooks, react, close: () => vite.close() };
}

const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 2 * 365 * 24 - 1 } };
const runtime = { client: {}, tracks: [track] } as unknown as WidgetRuntime;
const widget = (show = 'conditions') => ({ id: 'weather', kind: 'weather-by-region', trackIds: ['main'], time: { mode: 'campaign' }, options: { show } }) as unknown as WidgetSpec;
const choices = { dayHours: 24, regions: [], flags: [], modifiers: [], temperature: 'available', warnings: [] };
const reply = (first: number, last: number, fields: Record<string, unknown> = {}): WeatherReply =>
  ({ ...choices, window: { firstHour: first, lastHour: last, known: [], reads: [], series: [] }, ...fields }) as WeatherReply;
const flush = () => new Promise(resolve => setImmediate(resolve));

test('a folded day is read through the transient route after its dwell, superseded reads are aborted, and only finished days are kept', async () => {
  const fixture: Fixture = { loads: [], products: [], product: () => ({}) };
  host.__weatherHooks = fixture;
  const { hooks, react, close } = await mountHooks();
  mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  try {
    const days = () => react.render(() => hooks.useWeatherDays(widget(), runtime, true, 24));
    days().want(5);
    mock.timers.tick(hooks.DWELL_MS - 1);
    assert.equal(fixture.loads.length, 0, 'no read before the pointer rests');
    mock.timers.tick(1);
    assert.deepEqual(fixture.loads.map(load => load.request.params), [{ branchId: '1', firstHour: 120, lastHour: 143, daily: true, show: 'conditions' }]);
    assert.deepEqual(fixture.products, [], 'never through the shared product cache');
    days().want(6);
    assert.equal(fixture.loads[0].signal.aborted, true, 'a read the pointer left is aborted');
    mock.timers.tick(hooks.DWELL_MS);
    fixture.loads[1].resolve(reply(144, 167));
    await flush();
    assert.ok(days().get(6)?.data);
    days().want(6);
    mock.timers.tick(hooks.DWELL_MS);
    assert.equal(fixture.loads.length, 2, 'a finished day is kept');
    // A day still being prepared is shown, read again later, and never kept.
    days().want(7);
    mock.timers.tick(hooks.DWELL_MS);
    fixture.loads[2].resolve(reply(168, 191, { coverage: { firstHour: 0, lastHour: 170, complete: false }, pending: [{ firstHour: 171, lastHour: 191 }] }));
    await flush();
    assert.ok(days().get(7)?.data);
    days().want(7);
    mock.timers.tick(hooks.PREPARING_REREAD_MS - 1);
    assert.equal(fixture.loads.length, 3, 'not read again at once');
    mock.timers.tick(1);
    assert.equal(fixture.loads.length, 4, 'read again while the pointer stays, once preparation has had time to move');
    fixture.loads[3].resolve(reply(168, 191, { coverage: { firstHour: 0, lastHour: 180, complete: false }, pending: [{ firstHour: 181, lastHour: 191 }] }));
    await flush();
    days().want(7);
    days().hover(null);
    mock.timers.tick(hooks.PREPARING_REREAD_MS);
    assert.equal(fixture.loads.length, 4, 'not read again once the pointer has left');
    // A failed read is not kept: it is tried again when the pointer comes back.
    days().want(8);
    mock.timers.tick(hooks.DWELL_MS);
    fixture.loads[4].reject(new Error('Synthetic failure'));
    await flush();
    assert.equal(days().get(8)?.error?.message, 'Synthetic failure');
    days().want(8);
    mock.timers.tick(hooks.DWELL_MS);
    assert.equal(fixture.loads.length, 5, 'no retry loop while the pointer stays');
    days().want(9);
    mock.timers.tick(hooks.DWELL_MS);
    fixture.loads[5].resolve(reply(216, 239));
    await flush();
    days().want(8);
    mock.timers.tick(hooks.DWELL_MS);
    assert.equal(fixture.loads.length, 7, 'the failed day is read again on return');
    // Leaving the chart forgets a failed read too, so coming back reads it again.
    fixture.loads[6].reject(new Error('Synthetic failure'));
    await flush();
    assert.equal(days().get(8)?.error?.message, 'Synthetic failure');
    days().hover(8);
    assert.equal(days().get(8)?.error?.message, 'Synthetic failure', 'still on the failed day: no retry loop');
    days().hover(null);
    assert.equal(days().get(8), undefined);
    days().want(8);
    mock.timers.tick(hooks.DWELL_MS);
    assert.equal(fixture.loads.length, 8, 'the failed day is read again after the pointer left the chart');
    // At most the recent finished days stay.
    for (let day = 20; day < 20 + hooks.RECENT_DAYS; day++) {
      days().want(day);
      mock.timers.tick(hooks.DWELL_MS);
      fixture.loads.at(-1)!.resolve(reply(day * 24, day * 24 + 23));
      await flush();
    }
    assert.equal(days().get(6), undefined, 'the oldest day is dropped');
    assert.ok(days().get(20 + hooks.RECENT_DAYS - 1)?.data);
  } finally { mock.timers.reset(); delete host.__weatherHooks; await close(); }
});

test('a content change forgets the days read under the old content and drops the read in flight', async () => {
  const fixture: Fixture = { loads: [], products: [], product: () => ({}), content: 0 };
  host.__weatherHooks = fixture;
  const { hooks, react, close } = await mountHooks();
  mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  try {
    const days = () => react.render(() => hooks.useWeatherDays(widget(), runtime, true, 24));
    days().want(5);
    mock.timers.tick(hooks.DWELL_MS);
    fixture.loads[0].resolve(reply(120, 143, { temperature: 'refused' }));
    await flush();
    assert.ok(days().get(5)?.data);
    days().want(6);
    mock.timers.tick(hooks.DWELL_MS);
    fixture.content = 1;
    assert.equal(days().get(5), undefined, 'a day read under the old content is not shown');
    assert.equal(fixture.loads[1].signal.aborted, true, 'the read in flight under the old content is dropped');
    days().want(5);
    mock.timers.tick(hooks.DWELL_MS);
    assert.equal(fixture.loads.length, 3, 'the day is read again under the new content');
    fixture.loads[2].resolve(reply(120, 143));
    await flush();
    assert.equal(days().get(5)?.data?.temperature, 'available');
  } finally { mock.timers.reset(); delete host.__weatherHooks; await close(); }
});

test('the view settles before its days are asked, at the plot width in 256 pixel steps, within the zoom; the last reply stays meanwhile', async () => {
  const shown = reply(0, 2 * 365 * 24 - 1);
  const fixture: Fixture = { loads: [], products: [], product: request => request && fixture.products.length === 1 ? { data: shown } : {} };
  host.__weatherHooks = fixture;
  const { hooks, react, close } = await mountHooks();
  // The window width the view starts from, as a browser reports it.
  const events = globalThis as unknown as { addEventListener?: unknown; removeEventListener?: unknown };
  events.addEventListener ??= () => {}; events.removeEventListener ??= () => {};
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const extent = [0, 2 * 365 * 24 - 1] as const;
    const view = (spec = widget()) => react.render(() => hooks.useWeatherInView(spec, runtime, true, extent));
    const first = view();
    assert.equal(first.data, shown);
    first.onSummaryView({ from: 100.4, to: 600.2 * 24, width: 300 });
    mock.timers.tick(hooks.VIEW_SETTLE_MS - 1);
    view();
    assert.deepEqual(fixture.products.at(-1), fixture.products[0], 'nothing asked before the view settles');
    mock.timers.tick(1);
    const zoomed = view();
    const params = fixture.products.at(-1)!.params as { firstHour: number; lastHour: number; width: number };
    assert.deepEqual([params.firstHour, params.lastHour, params.width], [96, 601 * 24 - 1, 512], 'the whole days in view, at the stepped plot width');
    assert.equal(zoomed.data, shown, 'the last reply stays shown while the zoomed one loads');
    assert.equal(view(widget('temperature')).data, undefined, 'another view never shows it');
  } finally { mock.timers.reset(); delete host.__weatherHooks; await close(); }
});
