import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { PreparedFocusHour } from './preparedFocusData';

test('an obsolete focus page failure cannot replace a newer successful generation', async () => {
  type Slot = { value?: any; deps?: readonly unknown[]; cleanup?: () => void };
  const state = { cursor: 0, slots: [] as Slot[], effects: [] as (() => void)[],
    publish: undefined as ((value: unknown) => void) | undefined };
  const host = globalThis as typeof globalThis & { __focusGeneration?: typeof state; __focusGenerationHook?: (name: string, ...args: any[]) => any };
  host.__focusGeneration = state;
  host.__focusGenerationHook = (name, ...args) => {
    const slot = state.slots[state.cursor++] ??= {};
    if (name === 'useEffect') {
      const [effect, deps] = args;
      if (!slot.deps || deps.some((value: unknown, index: number) => !Object.is(value, slot.deps![index]))) {
        slot.deps = [...deps]; state.effects.push(() => { slot.cleanup?.(); slot.cleanup = effect(); });
      }
      return;
    }
    if (!('value' in slot)) slot.value = name === 'useRef' ? { current: args[0] } : args[0];
    return name === 'useRef' ? slot.value : [slot.value, (next: any) => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/dashboard/preparedSeries.ts': 'export const preparedSeriesQuery = (...args) => { globalThis.__focusGeneration.publish = args[8]; return new Promise(() => {}); };',
    '/national-focus/useFocusArtwork.ts': 'export const useFocusArtwork = () => ({ details: new Map(), warnings: [] }); export const useContinuousFocusArtwork = () => ({ read: new Map(), warnings: [], details: new Map() });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-generation', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/national-focus/PreparedFocusWidget.tsx')) return source.replace(/\b(useState|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, name) => `globalThis.__focusGenerationHook(${JSON.stringify(name)}, `);
      } }] });
  let rejectOld!: (reason: Error) => void;
  const oldPage = new Promise<unknown>((_resolve, reject) => { rejectOld = reject; });
  let pages = 0;
  const row: PreparedFocusHour = { schema: 6, slot: 1, countryId: 7, hour: 30, lastHour: 40, orderFirstHour: 10,
    current: null, continuous: null, paused: false, progressRaw: '0', dailyGainRaw: null,
    completed: { first: '0', count: '1', rows: '1' }, at: 35 };
  try {
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/national-focus/PreparedFocusWidget.tsx')).default;
    const client = { preparedQuery: async (_source: unknown, product: string) => {
      assert.equal(product, 'focus-order-page'); assert.ok(++pages <= 2);
      return pages === 1 ? oldPage : Promise.reject(new Error('Current page failed'));
    } };
    const props = { widget: { id: 'focus', kind: 'national-focus', trackIds: ['main'], countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'campaign' } },
      runtime: { client, clock: { getHour: () => 35 }, tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1' },
        branch: { id: '1', firstHour: 0, lastHour: 35, gaps: [] } }] }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => {
      state.cursor = 0; state.effects = [];
      const html = renderToStaticMarkup(createElement(Widget, props));
      for (const effect of state.effects) effect();
      return html;
    };
    const settle = async () => { for (let step = 0; step < 4; step++) await new Promise(resolve => setImmediate(resolve)); };
    render();
    state.publish!(row); await settle();
    assert.equal(pages, 1);
    // A newer reply has an empty, successfully loaded completion page.
    state.publish!({ ...row, completed: { first: '0', count: '0', rows: '0' } }); await settle();
    const accepted = render();
    assert.match(accepted, /No completed focuses recorded at this point/);
    rejectOld(new Error('Obsolete page failed')); await settle();
    assert.equal(render(), accepted, 'failure ownership matches success ownership');
    state.publish!({ ...row, progressRaw: '1' }); await settle();
    assert.match(render(), /data-detail="Current page failed"/);
    assert.equal(pages, 2, 'a current failure is still reported');
  } finally {
    rejectOld(new Error('Fixture closed'));
    for (const slot of state.slots) slot.cleanup?.();
    delete host.__focusGeneration; delete host.__focusGenerationHook; await vite.close();
  }
});
