import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { UnitLeadersData } from './generalsData';

test('leaderboard settles mounted artwork demand for every sort and preserves row motion', async () => {
  type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
  const scope = () => ({ dirty: true, slots: new Map<number, unknown>(),
    effects: new Map<number, Effect>(), pendingEffects: new Map<number, Effect>(),
    layouts: new Map<number, Effect>(), pendingLayouts: new Map<number, Effect>() });
  const widgetScope = scope(), tableScope = scope();
  let current = widgetScope, cursor = 0, now = 0, nextTimer = 0;
  const timers = new Map<number, { run: () => void; due: number }>();
  const changed = (before: readonly unknown[] | undefined, after: readonly unknown[]) => !before
    || before.length !== after.length || after.some((value, index) => !Object.is(value, before[index]));
  const commit = (held: Map<number, Effect>, pending: Map<number, Effect>) => {
    const effects = [...pending]; pending.clear();
    for (const [index] of effects) held.get(index)?.cleanup?.();
    for (const [index, effect] of effects) {
      const cleanup = effect.run();
      held.set(index, { ...effect, cleanup: typeof cleanup === 'function' ? cleanup : undefined });
    }
  };
  const data = (count: number): UnitLeadersData => ({ branchId: '1', firstHour: 100,
    people: Array.from({ length: count }, (_, index) => ({ characterId: index + 5, token: null, name: `Leader ${index}`,
      portraitHour: 100, portraitReference: String(index + 5) })),
    roles: [{ owner: 'synthetic', category: 1, rank: null, kind: 1, maximumSkill: 10, attributeNames: [null, null, null, null] }],
    traits: [], leaders: Array.from({ length: count }, (_, index) => ({ reference: String(index + 5),
      runs: [{ firstHour: 100, lastHour: 102, value: { person: index, role: 0, skill: 3 + index * 2, components: [1, 2, 3, 4], traits: [] } }],
      traitGains: [], commands: [], partialFrom: null,
      foughtHours: [100, ...Array(index + 3).fill(0)], wonHours: [100, ...Array(1 - Math.min(index, 1)).fill(0)],
      lostHours: [100, ...Array(Math.min(index, 1)).fill(0)] })),
    observed: [{ firstHour: 100, lastHour: 102, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
    resultsMissing: 'Result not recorded', navyCommandsMissing: 'Navy commands not recorded', warnings: [] });
  const fixture = {
    data: data(2), countries: new Set(['synthetic']), portraits: new Map<string, unknown>(),
    tokens: { motion: 'smooth', reducedMotion: false },
    labelAt: () => (tag: string) => ({ name: tag }),
    useState(initial: unknown) {
      const index = cursor++, owner = current;
      let state = owner.slots.get(index) as { value: unknown; set: (next: unknown) => void } | undefined;
      if (!state) {
        state = { value: typeof initial === 'function' ? initial() : initial, set: next => {
          const held = owner.slots.get(index) as { value: unknown };
          const value = typeof next === 'function' ? next(held.value) : next;
          if (!Object.is(value, held.value)) { held.value = value; owner.dirty = true; }
        } };
        owner.slots.set(index, state);
      }
      return [state.value, state.set] as const;
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!current.slots.has(index)) current.slots.set(index, { current: initial });
      return current.slots.get(index) as { current: unknown };
    },
    useId() { return fixture.useRef('synthetic-table').current; },
    useMemo(build: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, previous = current.slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (changed(previous?.deps, deps)) current.slots.set(index, { deps: [...deps], value: build() });
      return (current.slots.get(index) as { value: unknown }).value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return fixture.useMemo(() => callback, deps); },
    useReducer(reduce: (before: unknown, action: unknown) => unknown, initial: unknown) {
      const [value, set] = fixture.useState(initial);
      return [value, fixture.useMemo(() => (action: unknown) => set((before: unknown) => reduce(before, action)), [reduce, set])];
    },
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++;
      if (changed(current.effects.get(index)?.deps, deps)) current.pendingEffects.set(index, { deps: [...deps], run });
    },
    useLayoutEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++;
      if (changed(current.layouts.get(index)?.deps, deps)) current.pendingLayouts.set(index, { deps: [...deps], run });
    },
  };
  const host = globalThis as typeof globalThis & { __leaderMotionHooks?: typeof fixture };
  host.__leaderMotionHooks = fixture;
  const globals = ['window', 'ResizeObserver', 'performance'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    setTimeout(run: () => void, delay: number) { const id = ++nextTimer; timers.set(id, { run, due: now + delay }); return id; },
    clearTimeout(id: number) { timers.delete(id); },
  } });
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: class { observe() {} unobserve() {} disconnect() {} } });
  Object.defineProperty(globalThis, 'performance', { configurable: true, value: { now: () => now } });
  // The widget, portrait hook, Table and row-motion effects run together. Only
  // React's dispatcher, data transport and unmounted browser measurements are synthetic.
  const replacements: Record<string, string> = {
    '/widgets/generals/useLeaders.ts': `export const useLeaders = () => ({ data: globalThis.__leaderMotionHooks.data, retry() {} });
      export const useLeaderCountries = () => ({ countries: globalThis.__leaderMotionHooks.countries, requestSelection: { kind: 'all' }, group: {} });`,
    '/widgets/general-career/CareerView.tsx': 'export default () => null;',
    '/dashboard/countryNamesAt.ts': 'export { nameKey } from "./countryFacts.ts"; export const useCountryFactsAt = () => globalThis.__leaderMotionHooks.portraits;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: globalThis.__leaderMotionHooks.labelAt });',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 102;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000; export const useChartTokens = () => globalThis.__leaderMotionHooks.tokens;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'leader-motion-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.keys(replacements).find(key => path.endsWith(key));
        if (replacement) return replacements[replacement];
        if (['/generals/GeneralsWidget.tsx', '/generals/portraits.ts', '/table/Table.tsx', '/charts/useRowMotion.ts'].some(key => path.endsWith(key))) {
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__leaderMotionHooks;');
        }
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/generals/GeneralsWidget.tsx') as typeof import('./GeneralsWidget.tsx');
    const { default: Table } = await vite.ssrLoadModule('/ui/src/charts/table/Table.tsx') as typeof import('../../charts/table/Table.tsx');
    const { nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts') as typeof import('../../dashboard/countryFacts.ts');
    type Node = { type: unknown; props: Record<string, unknown> };
    const descendants = (value: unknown): Node[] => Array.isArray(value) ? value.flatMap(descendants)
      : value && typeof value === 'object' && 'props' in value ? [value as Node, ...descendants((value as Node).props.children), ...descendants((value as Node).props.main)] : [];
    let props = { widget: { id: 'generals', kind: 'generals', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'campaign' }, options: {} }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 102 } }],
        clock: { getHour: () => 102 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    let tableProps: Parameters<typeof Table>[0] | undefined;
    let nodes: Node[] = [];
    const settle = () => {
      const frames: Node[][] = [];
      for (let commits = 0; widgetScope.dirty || tableScope.dirty; commits++) {
        assert.ok(commits < 50, 'Generals/Table effects exceeded the React update-depth limit instead of settling');
        if (widgetScope.dirty) {
          current = widgetScope; cursor = 0; widgetScope.dirty = false;
          tableProps = descendants(Widget(props)).find(node => node.type === Table)?.props as unknown as Parameters<typeof Table>[0];
          assert.ok(tableProps, 'loaded leaders mount their shared Table'); tableScope.dirty = true;
        }
        if (tableScope.dirty) {
          current = tableScope; cursor = 0; tableScope.dirty = false;
          nodes = descendants(Table(tableProps!)); frames.push(nodes);
        }
        for (const owner of [tableScope, widgetScope]) commit(owner.layouts, owner.pendingLayouts);
        for (const owner of [tableScope, widgetScope]) commit(owner.effects, owner.pendingEffects);
      }
      return frames;
    };
    const rows = () => nodes.filter(node => node.props['data-table-row']);
    const keys = () => rows().filter(node => !node.props['aria-hidden']).map(node => node.props['data-table-row']);
    const refresh = () => { widgetScope.dirty = true; return settle(); };
    settle();
    assert.deepEqual(keys(), ['6', '5']);
    const descending: Record<string, string[]> = { skill: ['6', '5'], won: ['5', '6'], lost: ['6', '5'], unresolved: ['6', '5'], winRate: ['5', '6'] };
    for (const [column, expected] of Object.entries(descending)) for (const direction of ['descending', 'ascending']) {
      props = { ...props, widget: { ...props.widget, options: { tableSort: JSON.stringify([column, direction]) } } };
      refresh();
      assert.deepEqual(keys(), direction === 'descending' ? expected : [...expected].reverse(), `${column}/${direction}`);
      const art = { reference: '5', image: `synthetic portrait ${column}/${direction}`, missing: null, warnings: [] };
      fixture.portraits = new Map([[nameKey(100, '5'), art]]); refresh();
      assert.equal(nodes.find(node => node.props.art === art)?.props.art, art, 'new artwork still updates the rendered identity');
    }
    props = { ...props, widget: { ...props.widget, options: { tableSort: JSON.stringify(['skill', 'descending']) } } }; refresh();
    fixture.data = data(3);
    const arrived = refresh();
    assert.ok(arrived.flat().some(node => node.props['data-table-row'] === '7' && String(node.props.className).includes('is-entering')));
    assert.deepEqual(keys(), ['7', '6', '5']);
    assert.deepEqual(rows().map(node => node.props['aria-rowindex']), [2, 3, 4], 'new ordering retains the exact row slots');
    fixture.data = data(2); refresh();
    assert.ok(rows().some(node => node.props['data-table-row'] === '7' && String(node.props.className).includes('is-leaving')));
    now += 1000;
    for (const [id, timer] of [...timers]) if (timer.due <= now) { timers.delete(id); timer.run(); }
    settle();
    assert.deepEqual(keys(), ['6', '5']);
    assert.equal(rows().some(node => node.props['data-table-row'] === '7'), false, 'departed row drops after its fade');
  } finally {
    for (const owner of [tableScope, widgetScope]) for (const effect of [...owner.layouts.values(), ...owner.effects.values()]) effect.cleanup?.();
    await vite.close(); delete host.__leaderMotionHooks;
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
