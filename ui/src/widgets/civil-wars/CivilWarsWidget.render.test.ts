import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFile } from 'node:fs/promises';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, mountModule } from '../../dashboard/pendingGroups.fixture.ts';
import type { CivilWarsReply } from '../../generated/dashboard';

test('civil-war descriptions request every live side and dated heading beyond the viewport', async () => {
  const slots = new Map<number, unknown>(), effects = new Map<number, readonly unknown[]>();
  const pending: (() => void)[] = [];
  let cursor = 0;
  const useState = (initial: unknown) => {
    const index = cursor++;
    if (!slots.has(index)) slots.set(index, typeof initial === 'function' ? initial() : initial);
    return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)] as const;
  };
  const hooks = { useState, useMemo: (make: () => unknown) => make(), useCallback: (callback: unknown) => callback,
    useRef: (initial: unknown) => useState({ current: initial })[0],
    useReducer: (reduce: (state: unknown, action: unknown) => unknown, initial: unknown) => {
      const [value, set] = useState(initial); return [value, (action: unknown) => set((before: unknown) => reduce(before, action))];
    },
    useEffect: (run: () => void, deps: readonly unknown[]) => {
      const index = cursor++, before = effects.get(index);
      if (!before || before.length !== deps.length || deps.some((value, i) => !Object.is(value, before[i]))) {
        effects.set(index, deps); pending.push(run);
      }
    },
  };
  const data: CivilWarsReply = { branchId: BRANCH.id, lastHour: 1000, known: [{ firstHour: 0, lastHour: 1000 }], warnings: [],
    wars: Array.from({ length: 80 }, (_, index) => ({ id: `war-${index}`, firstHour: index + 1, lastHour: 1000,
      atWar: [{ firstHour: index + 1, lastHour: 1000 }], outcome: null, sides: [
        { tags: [{ hour: 0, tag: `Origin${index}` }], firstHour: index + 1, lastHour: 1000, parent: null, existedBefore: true, exit: null },
        { tags: [{ hour: index + 1, tag: `Rebel${index}` }], firstHour: index + 1, lastHour: 999, parent: 0, existedBefore: false, exit: null },
      ] })) };
  const state = { hooks, data, live: [] as string[], flags: [] as string[], dated: [] as { tag: string; hour: number }[],
    datedFlags: [] as { tag: string; hour: number }[], maxHours: 0, description: '', names: new Map<string, string>() };
  const host = globalThis as typeof globalThis & { __civilCoverage?: typeof state }; host.__civilCoverage = state;
  const source = (await readFile(new URL('./CivilWarsWidget.tsx', import.meta.url), 'utf8')).replace(
    /import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__civilCoverage.hooks;');
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/civil-wars/CivilWarsWidget.tsx', {
    '/widgets/civil-wars/CivilWarsWidget.tsx': source,
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__civilCoverage.data, retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: undefined });',
    '/widgets/army-size/headHour.ts': 'export const useHeadHour = () => 1000;',
    '/dashboard/widgetPerf.ts': 'export const useShownEachFrame = () => {};',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ color() {}, revision: 0 });',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
      export const useCountryFactsRead = (_client, _track, wanted, _active, _spec, maxHours) => {
        const state = globalThis.__civilCoverage; state.dated = wanted; state.maxHours = maxHours;
        return new Map(wanted.flatMap(item => { const key = nameKey(item.hour, item.tag), name = state.names.get(key);
          return name === undefined ? [] : [[key, { tag: item.tag, name, color: null }]]; }));
      };`,
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = (_client, _track, wanted) => { globalThis.__civilCoverage.datedFlags = wanted; return new Map(); };',
    '/dashboard/useCountriesAt.ts': `export const STYLES = {};
      export const countryReadName = (style, pending) => ({ name: style?.name ?? pending.name });
      export const useCountriesAt = (_client, _track, flags, hour, _active, names) => {
        const state = globalThis.__civilCoverage; state.live = names; state.flags = flags;
        return { country: tag => ({ name: state.names.get(JSON.stringify([hour, tag])) ?? (names.includes(tag) ? 'Loading…' : 'Name not known') }), revision: state.names.size };
      };`,
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ revision: 0 });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ surface: "#000000", text: "#ffffff", positive: "#00ff00" }); export const useViewportHeight = () => 1000;',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__civilCoverage.description = props.description; return null; };',
  });
  try {
    const props = { widget: { id: 'civil', kind: 'civil-wars', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, lastHour: 1000, gaps: [] } }],
        clock: { getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { cursor = 0; renderToStaticMarkup(createElement(module.default, props)); for (const run of pending.splice(0)) run(); };
    render(); render();
    assert.ok(state.live.includes('Origin0'), 'a never-visible live side has its name requested');
    assert.ok(state.dated.some(item => item.tag === 'Origin0' && item.hour === 1), 'the never-visible heading is named at the war start');
    assert.ok(state.dated.some(item => item.tag === 'Rebel0' && item.hour === 999), 'a former side is named at its own last read');
    assert.equal(state.dated.length, new Set(state.dated.map(item => JSON.stringify([item.hour, item.tag]))).size);
    assert.ok(state.maxHours >= new Set(state.dated.map(item => item.hour)).size, 'all description hours stay admitted beyond the default cache size');
    assert.ok(!state.flags.includes('Origin0') && !state.datedFlags.some(item => item.tag === 'Rebel0'), 'flag artwork stays in the viewport');
    assert.match(state.description, /Loading…/);
    for (const tag of state.live) state.names.set(JSON.stringify([1000, tag]), `Named ${tag} at 1000`);
    for (const item of state.dated) state.names.set(JSON.stringify([item.hour, item.tag]), `Named ${item.tag} at ${item.hour}`);
    render();
    assert.match(state.description, /Named Origin0 at 1/);
    assert.match(state.description, /Named Origin0 at 1000/);
    assert.match(state.description, /Named Rebel0 at 999/);
    assert.doesNotMatch(state.description, /Loading…|Name not known/);
  } finally { delete host.__civilCoverage; await close(); }
});
