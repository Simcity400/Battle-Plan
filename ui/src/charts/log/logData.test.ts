import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { anchoredScrollTop, cardHeightsAt, cardPageTarget, hasLogPartTip, keyTarget, logDate, logItems, logOffsets, logRowColumns, monthLabel, MONTH_HEIGHT, newCardKeys, newestFirst, ROW_HEIGHT, rowId, scrollToShow, shownMotion, tipRow, updateCardHeights, visibleRange } from './logData.ts';
import type { CardMeasurements, LogRow } from './logData';
import type { LogListProps } from './LogList';
import type { MovingRow } from '../rowMotion';
import { commitRows, EMPTY_HISTORY, shownRows } from '../rowMotion.ts';
import { LOG_OPTIONS, logSettings } from './logOptions.ts';
import { resolveOptions } from '../../dashboard/options.ts';

const hour = (year: number, month: number, day: number, at = 1) => engineHour({ year, month, day, hour: at }) as number;
const row = (key: string, at: number, by = false): LogRow<string> => ({ key, hour: at, by, item: key });

test('mixed log rows reserve optional tracks only for the rows that draw their content', () => {
  const sentence = 'var(--space-6) minmax(0, 1fr) auto';
  const meter = 'var(--space-6) minmax(0, 2fr) minmax(0, 1fr) auto';
  const rows = [row('ongoing', 10), row('ended', 20, true), row('unread', 15)];
  const columns = (row: LogRow<string>) => row.item === 'ongoing' ? meter : sentence;
  for (const months of [false, true]) {
    const items = logItems(rows, months).filter(item => item.kind === 'row');
    assert.deepEqual(items.map(item => logRowColumns(columns, item.row)), [meter, sentence, sentence]);
    assert.deepEqual(items.map(item => item.row.hour), [10, 20, 15]);
    assert.deepEqual(items.map(item => logRowColumns(sentence, item.row)), [sentence, sentence, sentence]);
  }
});

test('rows are newest first, and rows of one hour keep the order given', () => {
  const rows = [row('a', 10), row('b', 30), row('c', 20), row('d', 30)];
  assert.deepEqual(newestFirst(rows).map(item => item.key), ['b', 'd', 'c', 'a']);
});

test('month dividers precede each month’s first row, keyed by the month so they hold still', () => {
  const rows = newestFirst([row('a', hour(1941, 6, 22)), row('b', hour(1941, 6, 2)), row('c', hour(1941, 5, 30)), row('d', hour(1940, 12, 31))]);
  const items = logItems(rows, true);
  assert.deepEqual(items.map(item => item.kind === 'month' ? `[${item.label}]` : item.key), ['[Jun 1941]', 'a', 'b', '[May 1941]', 'c', '[Dec 1940]', 'd']);
  assert.equal(logItems(rows, false).length, 4);
  // The same month keeps its key whatever rows it holds.
  const again = logItems(newestFirst([row('x', hour(1941, 6, 30)), ...rows]), true);
  assert.equal(again[0].key, items[0].key);
  assert.equal(monthLabel(hour(1936, 1, 1)), 'Jan 1936');
  assert.equal(monthLabel(0), 'Undated');
});

test('offsets stack rows and dividers at their heights for either density', () => {
  const items = logItems(newestFirst([row('a', hour(1941, 6, 2)), row('b', hour(1941, 5, 2))]), true);
  assert.deepEqual(logOffsets(items, 'comfortable'), [0, MONTH_HEIGHT, MONTH_HEIGHT + ROW_HEIGHT.comfortable, 2 * MONTH_HEIGHT + ROW_HEIGHT.comfortable,
    2 * MONTH_HEIGHT + 2 * ROW_HEIGHT.comfortable]);
  assert.equal(logOffsets(items, 'compact').at(-1), 2 * MONTH_HEIGHT + 2 * ROW_HEIGHT.compact);
  assert.deepEqual(logOffsets([], 'compact'), [0]);
});

test('pinned running rows retain unique month sections before newest-first history', () => {
  const items = logItems([row('running', hour(1941, 5, 30)), row('finished', hour(1941, 6, 2)), row('older', hour(1941, 5, 2))], true);
  assert.equal(new Set(items.map(item => item.key)).size, items.length);
  assert.deepEqual(items.filter(item => item.kind === 'month').map(item => item.label), ['May 1941', 'Jun 1941', 'May 1941']);
});

test('only the items in view are drawn, with a margin each way', () => {
  const offsets = Array.from({ length: 101 }, (_, index) => index * 10);
  assert.deepEqual(visibleRange(offsets, 0, 50, 2), [0, 7]);
  assert.deepEqual(visibleRange(offsets, 205, 50, 2), [18, 28]);
  // Scrolled past the end, or a list with nothing in it.
  assert.deepEqual(visibleRange(offsets, 5000, 50, 2), [97, 100]);
  assert.deepEqual(visibleRange([0], 0, 50), [0, 0]);
});

test('reaching a row scrolls as little as needed, clear of a held month divider', () => {
  const offsets = Array.from({ length: 21 }, (_, index) => index * 10);
  // Already whole in view: no scroll.
  assert.equal(scrollToShow(offsets, 3, 0, 50), 0);
  // Below the view: its bottom meets the view's bottom.
  assert.equal(scrollToShow(offsets, 8, 0, 50), 40);
  // Above the view: its top clears the divider held over the rows.
  assert.equal(scrollToShow(offsets, 5, 100, 50, 24), 26);
  assert.equal(scrollToShow(offsets, 0, 100, 50, 24), 0);
});

test('keys move the keyboard row within the list', () => {
  assert.equal(keyTarget('ArrowDown', 0, 5, 3), 1);
  assert.equal(keyTarget('ArrowDown', 4, 5, 3), 4);
  assert.equal(keyTarget('ArrowUp', 0, 5, 3), 0);
  assert.equal(keyTarget('PageDown', 1, 5, 3), 4);
  assert.equal(keyTarget('PageUp', 4, 5, 3), 1);
  assert.equal(keyTarget('Home', 3, 5, 3), 0);
  assert.equal(keyTarget('End', 0, 5, 3), 4);
  assert.equal(keyTarget('a', 0, 5, 3), null);
  assert.equal(keyTarget('ArrowDown', 0, 0, 3), null);
});

test('a row’s date shows its day and month over its year', () => {
  assert.deepEqual(logDate(hour(1943, 12, 18, 4)), { day: '18 Dec', year: '1943' });
  assert.deepEqual(logDate(12), { day: 'Hour 12', year: '' });
});

test('log settings use their defaults and fall back on bad values', () => {
  assert.deepEqual(logSettings(resolveOptions(LOG_OPTIONS, {})), { density: 'comfortable', months: false, animate: true, chips: true });
  assert.deepEqual(logSettings(resolveOptions(LOG_OPTIONS, { density: 'compact', months: true, rowAnimation: false, chips: false })),
    { density: 'compact', months: true, animate: false, chips: false });
  assert.equal(logSettings(resolveOptions(LOG_OPTIONS, { density: 'tiny' })).density, 'comfortable');
});

test('without animation a row that is gone goes at once, never left over its replacement', () => {
  const moving = (key: string, slot: number, phase: MovingRow<string>['phase']): MovingRow<string> => ({ key, item: key, slot, phase, since: 0 });
  const rows = [moving('new', 0, 'entering'), moving('b', 1, 'staying'), moving('old', 0, 'leaving'), moving('far', 40, 'staying')];
  const keys = (list: MovingRow<string>[]) => list.map(item => item.key);
  assert.deepEqual(keys(shownMotion(rows, 0, 10, true)), ['new', 'b', 'old']);
  assert.deepEqual(keys(shownMotion(rows, 0, 10, false)), ['new', 'b']);
  // The keyboard's row stays drawn out of view, but a gone row is never kept.
  assert.deepEqual(keys(shownMotion(rows, 0, 10, false, item => item.key === 'far' || item.key === 'old')), ['new', 'b', 'far']);
});

test('a tooltip follows what its row is about across the row’s keys, and goes with it', () => {
  const stage = (key: string, id?: string): LogRow<string> => ({ key, id, hour: 1, item: key });
  assert.equal(rowId(stage('p:0', 'p')), 'p');
  assert.equal(rowId(stage('plain')), 'plain');
  const before = new Map([stage('p:0', 'p')].map(row => [rowId(row), row]));
  const after = new Map([stage('p:1', 'p')].map(row => [rowId(row), row]));
  // The project built a level: its row re-entered as p:1, and the tooltip now shows that row.
  assert.equal(tipRow({ id: 'p' }, before)?.key, 'p:0');
  assert.equal(tipRow({ id: 'p' }, after)?.key, 'p:1');
  // Filtered out or gone: the tooltip is hidden, not left describing an old state.
  assert.equal(tipRow({ id: 'p' }, new Map()), null);
  assert.equal(tipRow(null, after), null);
});


test('window release preserves a scrolled row while the top follows new entries', async () => {
  const { anchoredScrollTop, logItems, logOffsets } = await import('./logData.ts');
  const rows = Array.from({ length: 100 }, (_, i) => ({ key: String(i), hour: 100 - i, item: i }));
  const before = logItems(rows, false), after = logItems(rows.slice(20), false);
  const oldOffsets = logOffsets(before, 'compact'), newOffsets = logOffsets(after, 'compact');
  Object.defineProperty(oldOffsets, 'findLastIndex', { value: undefined });
  assert.equal(anchoredScrollTop(before, oldOffsets, after, newOffsets, oldOffsets[30] + 3), newOffsets[10] + 3);
  assert.equal(anchoredScrollTop(before, oldOffsets, after, newOffsets, 0), 0);
});


test('a new interface font discards offscreen card measurements at unchanged width', () => {
  const measured = { width: 400, density: 'comfortable' as const, font: 'First', heights: new Map([['old', 100]]) };
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', 'First').get('old'), 100);
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', 'Second').size, 0);
  const next = updateCardHeights(measured, 400, 'comfortable', [{ key: 'drawn', height: 120 }], 'Second');
  assert.deepEqual([...next.heights], [['drawn', 120]]);
});


test('card heights stack with month dividers and window by their measured layout pixels', () => {
  const rows = [row('a', hour(1941, 6, 2)), row('b', hour(1941, 6, 1)), row('c', hour(1941, 5, 2))];
  const heights = new Map([['a', 150], ['b', 310], ['c', 90]]);
  const items = logItems(rows, true);
  const offsets = logOffsets(items, 'comfortable', at => heights.get(at.key)!);
  assert.deepEqual(offsets, [0, 24, 174, 484, 508, 598]);
  assert.deepEqual(visibleRange(offsets, 200, 200, 0), [2, 3]);
  assert.deepEqual(visibleRange(offsets, 460, 100, 0), [2, 5]);
  // A new measurement changes the next offset, without scaling for screen zoom.
  heights.set('b', 350);
  assert.equal(logOffsets(items, 'compact', at => heights.get(at.key)!).at(-1), 638);
  assert.deepEqual(logOffsets(logItems(rows, false), 'compact', () => Number.NaN), [0, 28, 56, 84]);
});


test('layout width and density changes discard old wrapping heights, including unseen cards', () => {
  const measured = { width: 600, density: 'comfortable' as const, heights: new Map([['drawn', 140], ['unseen', 160]]) };
  assert.equal(cardHeightsAt(measured, 600, 'comfortable'), measured.heights);
  assert.equal(cardHeightsAt(measured, 400, 'comfortable').size, 0);
  assert.equal(cardHeightsAt(measured, 600, 'compact').size, 0);
  assert.equal(updateCardHeights(measured, 600, 'comfortable', [{ key: 'drawn', height: 140 }]), measured);
  const resized = updateCardHeights(measured, 400, 'comfortable', [{ key: 'drawn', height: 210 }]);
  assert.deepEqual([...resized.heights], [['drawn', 210]]);
  const denser = updateCardHeights(resized, 400, 'compact', [{ key: 'drawn', height: 120 }]);
  assert.deepEqual([...denser.heights], [['drawn', 120]]);
  const more = updateCardHeights(denser, 400, 'compact', [{ key: 'new', height: 170 }, { key: 'invalid', height: Number.NaN }]);
  assert.deepEqual([...more.heights], [['drawn', 120], ['new', 170]]);
  assert.deepEqual([...updateCardHeights(more, 350, 'compact', []).heights], []);
});


test('a descendant tooltip takes hover from its log row, including nested chip flags', () => {
  type Part = { parentElement: Part | null; getAttribute(name: string): string | null };
  const part = (parentElement: Part | null, tooltip: string | null = null): Part => ({ parentElement, getAttribute: name => name === 'data-tooltip' ? tooltip : null });
  const row = part(null);
  const plain = part(row);
  const chip = part(row, 'Remaining state names');
  const flag = part(chip);
  assert.equal(hasLogPartTip(chip, row), true);
  assert.equal(hasLogPartTip(flag, row), true);
  assert.equal(hasLogPartTip(plain, row), false);
  assert.equal(hasLogPartTip(part(row, ''), row), false);
  assert.equal(hasLogPartTip(part(chip, ''), row), false);
  assert.equal(hasLogPartTip(row, row), false);
  assert.equal(hasLogPartTip(null, row), false);
});


test('a card taller than the viewport is revealed at its heading without alternating ends', () => {
  const offsets = [0, 100, 600, 700];
  assert.equal(scrollToShow(offsets, 1, 0, 200), 100);
  assert.equal(scrollToShow(offsets, 1, 100, 200), 100);
  assert.equal(scrollToShow(offsets, 1, 400, 200, 24), 76);
  assert.equal(scrollToShow(offsets, 0, 200, 80, 24), 0);
});


test('paging cards moves one viewport through row heights, skipping month dividers', () => {
  const offsets = [0, 24, 124, 424, 448, 498, 578];
  const slots = [1, 2, 4, 5];
  assert.equal(cardPageTarget(offsets, slots, 0, 400, 1), 1);
  assert.equal(cardPageTarget(offsets, slots, 0, 440, 1), 2);
  assert.equal(cardPageTarget(offsets, slots, 1, 100, 1), 2);
  assert.equal(cardPageTarget(offsets, slots, 3, 400, -1), 1);
  assert.equal(cardPageTarget(offsets, slots, 1, 20, -1), 0);
  assert.equal(cardPageTarget(offsets, slots, 3, 400, 1), 3);
  assert.equal(cardPageTarget(offsets, [], 0, 400, 1), 0);
});


test('cards enter only once; existing cards never re-enter after scroll or backward seek', () => {
  const wrap = (keys: string[]) => keys.map(key => ({ key, item: key }));
  const first = wrap(['old']);
  let history = commitRows(EMPTY_HISTORY, first, 0, 450);
  const known = new Set(['old']);
  assert.deepEqual(newCardKeys(shownRows(history, first, 0, 450), known), []);
  const arriving = wrap(['new', 'old']);
  const moving = shownRows(history, arriving, 0, 450);
  assert.deepEqual(newCardKeys(moving, known), ['new']);
  for (const item of arriving) known.add(item.key);
  history = commitRows(history, arriving, 0, 450);
  // The shared motion phase remains entering while these items are unchanged.
  assert.equal(shownRows(history, arriving, 0, 450)[0].phase, 'entering');
  assert.deepEqual(newCardKeys(shownRows(history, arriving, 0, 450), known), []);
  history = commitRows(history, first, 1000, 450);
  history = commitRows(history, first, 2000, 450);
  assert.deepEqual(newCardKeys(shownRows(history, arriving, 2000, 450), known), []);
  // A first arrival after a committed empty log still enters while Live is on.
  const empty = commitRows(EMPTY_HISTORY, wrap([]), 0, 450);
  assert.deepEqual(newCardKeys(shownRows(empty, first, 0, 450), new Set()), ['old']);
});


test('cards added with motion disabled never replay when full motion is restored', () => {
  const rows: MovingRow<string>[] = [{ key: 'new', item: 'new', slot: 0, phase: 'entering', since: 0 }];
  const known = new Set<string>();
  assert.deepEqual(newCardKeys(rows, known, false), []);
  for (const row of rows) known.add(row.key);
  assert.deepEqual(newCardKeys(rows, known, true), []);
});


test('5,000 variable-height cards retain every offset and window only the rows in view', () => {
  const count = 5_000;
  const items = logItems(Array.from({ length: count }, (_, index) => row(String(index), count - index)), false);
  const offsets = logOffsets(items, 'comfortable', at => 110 + at.hour % 5 * 28);
  assert.equal(items.length, count);
  assert.equal(offsets.length, count + 1);
  assert.equal(offsets.at(-1), 830_000);
  const [first, last] = visibleRange(offsets, offsets[count / 2], 500);
  assert.ok(last - first < 30);
  assert.deepEqual([first, last], [2_494, 2_509]);
});


test('card logs remove future cards immediately on a backward seek or Live cutoff', () => {
  const rows: MovingRow<string>[] = [
    { key: 'reached', item: 'reached', slot: 0, phase: 'staying', since: 0 },
    { key: 'future', item: 'future', slot: 0, phase: 'leaving', since: 0 },
  ];
  assert.deepEqual(shownMotion(rows, 0, 10, true, () => true, 'card').map(row => row.key), ['reached']);
  assert.deepEqual(shownMotion(rows, 0, 10, true).map(row => row.key), ['reached', 'future']);
});


test('measured cards preserve the reached heading when overscan heights change above it', () => {
  const items = logItems([row('above', 3), row('reached', 2), row('below', 1)], false);
  const before = logOffsets(items, 'comfortable', () => 100);
  const heights = new Map([['above', 180], ['reached', 140], ['below', 100]]);
  const after = logOffsets(items, 'comfortable', at => heights.get(at.key)!);
  const top = anchoredScrollTop(items, before, items, after, 112);
  assert.equal(top, 192);
  assert.deepEqual(visibleRange(after, top, 60, 0), [1, 2]);
  assert.equal(anchoredScrollTop(items, before, items, after, 0), 0);
});

test('a changed type scale invalidates offscreen wrapping at the same font family and width', () => {
  const first = JSON.stringify(['Synthetic family', 11, 12, 13]);
  const larger = JSON.stringify(['Synthetic family', 12, 14, 16]);
  const measured = { width: 400, density: 'comfortable' as const, font: first, heights: new Map([['unseen', 100]]) };
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', first).size, 1);
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', larger).size, 0);
  assert.deepEqual([...updateCardHeights(measured, 400, 'comfortable', [{ key: 'drawn', height: 140 }], larger).heights], [['drawn', 140]]);
});


test('a shrinking measured card retains its anchor inside the new height', () => {
  const items = logItems([row('reached', 2), row('next', 1)], false);
  const before = [0, 300, 600], after = [0, 100, 400];
  const top = anchoredScrollTop(items, before, items, after, 250);
  assert.equal(top, 99);
  assert.deepEqual(visibleRange(after, top, 1, 0), [0, 1]);
  assert.equal(anchoredScrollTop(items, before, items, [0, 0.5, 300], 250), 0);
});


test('card focus and delayed page navigation preserve reached tooltip ownership', async () => {
  const { createServer } = await import('vite');
  const slots = new Map<number, unknown>(), frames: (() => void)[] = [];
  let cursor = 0;
  type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
  const effects = new Map<number, Effect>(), pendingEffects = new Map<number, Effect>();
  const layouts = new Map<number, Effect>(), pendingLayouts = new Map<number, Effect>();
  let measureLayout = false;
  const commitEffects = (store: Map<number, Effect>, pending: Map<number, Effect>) => {
    const changed = [...pending];
    pending.clear();
    for (const [index] of changed) store.get(index)?.cleanup?.();
    for (const [index, effect] of changed) {
      const cleanup = effect.run();
      store.set(index, { ...effect, cleanup: typeof cleanup === 'function' ? cleanup : undefined });
    }
  };
  const commit = () => { commitEffects(layouts, pendingLayouts); commitEffects(effects, pendingEffects); };
  const fixture = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, typeof initial === 'function' ? initial() : initial);
      return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index) as { current: unknown };
    },
    useId() { return fixture.useRef(`log-${cursor}`).current; },
    useMemo(build: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, previous = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!previous || deps.length !== previous.deps.length || deps.some((dep, at) => !Object.is(dep, previous.deps[at]))) {
        slots.set(index, { deps: [...deps], value: build() });
      }
      return (slots.get(index) as { value: unknown }).value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return fixture.useMemo(() => callback, deps); },
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++, previous = effects.get(index);
      if (!previous || deps.length !== previous.deps.length || deps.some((dep, at) => !Object.is(dep, previous.deps[at]))) {
        pendingEffects.set(index, { deps: [...deps], run });
      }
    },
    useLayoutEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++, previous = layouts.get(index);
      if (measureLayout && (!previous || deps.length !== previous.deps.length || deps.some((dep, at) => !Object.is(dep, previous.deps[at])))) {
        pendingLayouts.set(index, { deps: [...deps], run });
      }
    },
  };
  const host = globalThis as typeof globalThis & { __logCardHooks?: typeof fixture };
  host.__logCardHooks = fixture;
  const globals = ['document', 'CSS', 'requestAnimationFrame', 'Element', 'ResizeObserver', 'window'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  class Part {
    readonly text: string | null;
    readonly parentElement: Part | null;
    constructor(text: string | null = null, parentElement: Part | null = null) {
      this.text = text;
      this.parentElement = parentElement;
    }
    getAttribute(name: string) { return name === 'data-tooltip' ? this.text : null; }
  }
  Object.defineProperty(globalThis, 'Element', { configurable: true, value: Part });
  const observers: Observer[] = [];
  class Observer {
    readonly targets: unknown[] = [];
    readonly changed: (entries: { target: unknown }[]) => void;
    constructor(changed: (entries: { target: unknown }[]) => void) {
      this.changed = changed;
      observers.push(this);
    }
    observe(target: unknown) { this.targets.push(target); this.changed([{ target }]); }
    unobserve() {} disconnect() {}
  }
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: Observer });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { setTimeout: () => 0, clearTimeout() {} } });
  const chip = new Part();
  const page: { activeElement: unknown } = { activeElement: chip };
  Object.defineProperty(globalThis, 'document', { configurable: true, value: page });
  Object.defineProperty(globalThis, 'CSS', { configurable: true, value: { escape: (value: string) => value } });
  Object.defineProperty(globalThis, 'requestAnimationFrame', { configurable: true, value: (frame: () => void) => { frames.push(frame); return frames.length; } });
  // Only React's dispatcher and environmental hooks are synthetic. The LogList
  // component, navigation/effects, loading demand and tooltip store run. Layout
  // measurement is controlled by the scroller rather than a browser observer.
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'log-card-focus-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/log/LogList.tsx') || path.endsWith('/charts/useRowMotion.ts')) {
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__logCardHooks;');
        }
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export function useContextMenu() { return { menu: null, open() {} }; }';
        if (path.endsWith('/charts/useChartTokens.ts')) return "export function useViewportHeight() { return 600; } export function useChartTokens() { return { font: 'Synthetic', fontSize: { small: 11, body: 12, title: 13 }, reducedMotion: false }; }";
      },
    }] });
  try {
    const { default: LogList } = await vite.ssrLoadModule('/ui/src/charts/log/LogList.tsx') as typeof import('./LogList.tsx');
    const { tooltipStore, hideTooltip } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts') as typeof import('../tooltip.ts');
    type Node = { ref?: (node: unknown) => void; props: Record<string, unknown> };
    const descendants = (value: unknown): Node[] => Array.isArray(value) ? value.flatMap(descendants)
      : value && typeof value === 'object' && 'props' in value ? [value as Node, ...descendants((value as Node).props.children)] : [];
    const reached: (string | null)[] = [];
    let props: LogListProps<string> = { rows: [row('first', 3), row('middle', 2), row('last', 1)], label: 'Cards', density: 'comfortable' as const,
      months: false, animate: false, layout: 'card' as const, rowHeight: () => 100, renderRow: (at: LogRow<string>) => at.item,
      describe: (at: LogRow<string>) => at.key, tooltip: (at: LogRow<string>) => [{ title: at.key, rows: [] }],
      onTooltip: (at: LogRow<string> | null) => reached.push(at?.key ?? null) };
    const render = () => { cursor = 0; return descendants(LogList(props)); };
    const flush = () => { for (const frame of frames.splice(0)) frame(); };
    let nodes = render();
    let list = nodes.find(node => node.props.role === 'listbox')!;
    const scroller = { scrollTop: 0, clientHeight: 200, offsetHeight: 200, clientWidth: 400, contains: (node: unknown) => node === chip,
      querySelector: () => ({ getBoundingClientRect: () => ({ right: 400, top: 0, height: 100 }) }),
      focus(): void { page.activeElement = scroller; (list.props.onFocus as (event: unknown) => void)({ target: scroller, currentTarget: scroller }); } };
    ((list.props.ref ?? list.ref) as (node: unknown) => void)(scroller);
    nodes = render(); list = nodes.find(node => node.props.role === 'listbox')!;
    (list.props.onKeyDown as (event: unknown) => void)({ key: 'End', preventDefault() {} });
    flush();
    nodes = render();
    assert.deepEqual(nodes.filter(node => node.props.role === 'option' && node.props['aria-selected']).map(node => node.props['data-log-id']), ['last']);
    assert.equal(reached.at(-1), 'last');
    assert.equal(tooltipStore.get()?.content.title, 'last');
    assert.equal(nodes.some(node => node.props.className === 'log-when'), false);
    commit();

    const wanted: string[][] = [];
    const visible: string[][] = [];
    const pendingPage = { key: 'older', start: 0, end: 100, count: 3 };
    const loadedPage = { ...pendingPage, rows: [row('A0', 90), row('A1', 80), row('A2', 70)] };
    const currentPage = { key: 'current', start: 101, end: 200, count: 1, rows: [row('B', 150)] };
    props = { ...props, rows: undefined, layout: 'row', rowHeight: undefined, pages: [currentPage, pendingPage], playhead: null,
      onWantedPages: keys => wanted.push([...keys]), onVisible: rows => visible.push(rows.map(at => at.key)) };
    const draw = () => { nodes = render(); list = nodes.find(node => node.props.role === 'listbox')!; commit(); };
    const key = (name: string) => (list.props.onKeyDown as (event: unknown) => void)({ key: name, preventDefault() {} });
    draw();
    key('End'); draw();
    assert.equal(tooltipStore.get(), null);
    assert.equal(reached.at(-1), null);
    // The list tells its owner what it shows and wants with the next frame.
    flush();
    assert.deepEqual(wanted.at(-1), ['older', 'current']);
    assert.deepEqual(visible.at(-1), ['B']);
    // A reached pointer row cancels the unloaded End destination. Its eventual
    // arrival cannot take the tooltip or re-admit the released detail request.
    const b = nodes.find(node => node.props['data-log-id'] === 'B')!;
    (b.props.onPointerMove as (event: unknown) => void)({ target: new Part(), currentTarget: new Part(), clientX: 20, clientY: 30 });
    draw();
    props = { ...props, pages: [currentPage, loadedPage] };
    draw(); flush(); draw();
    assert.equal(tooltipStore.get()?.content.title, 'B');
    assert.equal(reached.at(-1), 'B');
    assert.equal(nodes.some(node => node.props['data-log-id'] === 'A2' && node.props['aria-selected']), false);

    for (const takeover of ['blur', 'Escape', 'part'] as const) {
      props = { ...props, pages: [currentPage, pendingPage] };
      draw(); key('End'); draw();
      if (takeover === 'blur') (list.props.onBlur as () => void)();
      else if (takeover === 'Escape') key('Escape');
      else {
        const rowElement = new Part(), part = new Part('A part owns this tooltip', rowElement);
        const reachedRow = nodes.find(node => node.props['data-log-id'] === 'B')!;
        (reachedRow.props.onPointerMove as (event: unknown) => void)({ target: part, currentTarget: rowElement, clientX: 20, clientY: 30 });
      }
      draw();
      const beforeArrival = reached.length;
      props = { ...props, pages: [currentPage, loadedPage] };
      draw(); flush(); draw();
      assert.equal(tooltipStore.get(), null, takeover);
      assert.equal(reached.slice(beforeArrival).includes('A2'), false, takeover);
      assert.equal(nodes.some(node => node.props['data-log-id'] === 'A2' && node.props['aria-selected']), false, takeover);
    }
    // An uncancelled destination still completes at its filtered native offset.
    props = { ...props, pages: [currentPage, pendingPage] };
    draw(); key('End'); draw();
    props = { ...props, pages: [currentPage, { ...loadedPage, rows: loadedPage.rows.slice(0, 2) }] };
    draw(); flush(); draw();
    assert.equal(tooltipStore.get()?.content.title, 'A1');
    assert.equal(reached.at(-1), 'A1');
    assert.equal(nodes.some(node => node.props['data-log-id'] === 'A1' && node.props['aria-selected']), true);
    // Commit the real card measurement and entry effects against controlled DOM
    // heights. Released pages must release both native key and height retention.
    measureLayout = true;
    const cardPage = { key: 'card window', start: 0, end: 100, count: 1, rows: [row('held card', 80)] };
    props = { ...props, layout: 'card', rowHeight: () => 100, pages: [cardPage] };
    draw();
    const attach = (id: string) => {
      const node = nodes.find(node => node.props['data-log-id'] === id)!;
      const classes = new Set<string>();
      const element = { dataset: { logKey: id }, offsetHeight: 150,
        classList: { add: (name: string) => classes.add(name), remove: (name: string) => classes.delete(name) },
        addEventListener() {}, removeEventListener() {} };
      ((node.props.ref ?? node.ref) as (node: unknown) => void)(element);
      return (value: unknown) => ((node.props.ref ?? node.ref) as (node: unknown) => void)(value);
    };
    const detach = attach('held card');
    const releasedObserver = observers.at(-1)!;
    draw();
    const retained = () => [...slots.values()].find(value => value && typeof value === 'object' && 'heights' in value) as CardMeasurements;
    const known = () => [...slots.values()].map(value => (value as { current?: unknown } | null)?.current).find(value => value instanceof Set) as Set<string>;
    assert.equal(retained().heights.get('held card'), 150);
    assert.equal(known().has('held card'), true);
    detach(null);
    props = { ...props, pages: [{ ...cardPage, rows: undefined },
      { key: 'next window', start: 101, end: 200, count: 1, rows: [row('next card', 150)] }] };
    draw(); const detachNext = attach('next card'); draw();
    releasedObserver.changed([{ target: releasedObserver.targets[0] }]);
    assert.deepEqual([...retained().heights], [['next card', 150]]);
    assert.equal(known().has('held card'), false);
    assert.equal(known().has('next card'), true);
    detachNext(null);
    props = { ...props, pages: [] };
    draw(); draw();
    assert.equal(retained().heights.size, 0);
    assert.equal(known().size, 0);
    const opaqueKeys = ['actor A', 'actor_A', 'actor/%', 'actor/??', 'actor/\ud800'];
    props = { ...props, pages: undefined, rows: opaqueKeys.map((key, index) => row(key, 10 - index)) };
    draw(); flush(); draw();
    const options = nodes.filter(node => node.props.role === 'option');
    assert.equal(options.length, opaqueKeys.length);
    assert.equal(new Set(options.map(node => node.props.id)).size, opaqueKeys.length, 'opaque content keys retain distinct DOM IDs');
    assert.ok(options.every(node => !/\s/.test(String(node.props.id))));
    key('Home'); draw();
    assert.equal(list.props['aria-activedescendant'], options[0].props.id);
    for (const effect of [...layouts.values(), ...effects.values()]) effect.cleanup?.();
    assert.deepEqual(wanted.at(-1), []);
    assert.equal(reached.at(-1), null);
    const owner = tooltipStore.get()?.owner;
    if (owner) hideTooltip(owner);
  } finally {
    await vite.close();
    delete host.__logCardHooks;
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});


test('paged card measurement retains only held keys through 5,000 window evictions', () => {
  let measured: CardMeasurements = { width: 400, density: 'comfortable', heights: new Map() };
  for (let window = 0; window < 5_000; window++) {
    const key = `card ${window}`, held = new Set([key]);
    measured = updateCardHeights(measured, 400, 'comfortable', [{ key, height: 100 }, { key: 'released stale node', height: 200 }], '', held);
    assert.deepEqual([...measured.heights], [[key, 100]]);
  }
  // Ordinary card logs still retain estimates for their offscreen rows.
  measured = updateCardHeights(measured, 400, 'comfortable', [{ key: 'ordinary', height: 140 }]);
  assert.equal(measured.heights.size, 2);
});
