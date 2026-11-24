import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { amountText, arcTargets, findNode, foldTree, foldView, groupTree, drawable, hasCorrections, keyRings, keyedSlice, labelFits, notMeasured, pathTo, spokenAmount, sliceAt, listed, sliceColours, sliceTooltip, sunGeometry, sunburstSummary, sweepOf, walk } from './layout.ts';
import type { SunburstItem, SunburstNode } from './layout.ts';

const key = (id: string) => ({ id, label: id.toUpperCase() });
const item = (path: string[], value: number | null, extra: Partial<SunburstItem> = {}): SunburstItem => ({ path: path.map(key), value, ...extra });
const ROOT = { id: 'all', label: 'All' };

test('aggregate payment counts add through grouped and hidden levels without contributor IDs', () => {
  const parts = [
    item(['category', 'first'], 10, { count: 7000 }),
    item(['category', 'second'], 20, { count: 8000 }),
    item(['category', 'future'], null, { count: 9000, present: false, final: 30 }),
  ];
  for (const levels of [[true, true], [true, false], [false, true]]) {
    const tree = groupTree(parts, ROOT, levels);
    assert.equal(tree.count, 15000);
    assert.equal(tree.counted.size, 0);
    assert.equal(tree.aggregateCount, 15000);
    assert.equal(tree.children.reduce((total, node) => total + node.count, 0), 15000);
  }
});

test('aggregate counts and counted-ID unions stay exact through Other folding', () => {
  const tree = foldTree(groupTree([
    item(['large'], 100, { counted: ['shared'], count: 2 }),
    item(['first'], 10, { counted: ['shared', 'first'], count: 3 }),
    item(['second'], 5, { counted: ['shared', 'second'], count: 7 }),
  ], ROOT), 2);
  const other = tree.children.find(node => node.other)!;
  assert.equal(tree.count, 15);
  assert.deepEqual([...tree.counted].sort(), ['first', 'second', 'shared']);
  assert.equal(other.count, 13);
  assert.equal(other.aggregateCount, 10);
  assert.deepEqual([...other.counted].sort(), ['first', 'second', 'shared']);
  assert.equal(other.children.reduce((total, node) => total + node.aggregateCount, 0), 10);
});

test('an unfolded 5000-country legend mounts only its viewport and formats no full tooltips in render', async () => {
  let tooltips = 0;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'sunburst-window-theme', enforce: 'pre', transform(_source, id) {
      if (id.replaceAll('\\', '/').endsWith('/charts/useChartTokens.ts')) return "export const useChartTokens = () => ({ categories: ['#123'], text: '#fff', surface: '#000', fontSize: { small: 10, body: 12 } });";
    } }] });
  try {
    const { default: Sunburst } = await vite.ssrLoadModule('/ui/src/charts/sunburst/Sunburst.tsx') as typeof import('./Sunburst.tsx');
    const html = renderToStaticMarkup(createElement(Sunburst, {
      label: 'Countries', root: ROOT, active: true, fold: false,
      items: Array.from({ length: 5000 }, (_, index) => ({
        path: [{ ...key(`country-${index}`), countryFlag: { flag: `country-${index}.png` } }],
        value: index + 1, exact: BigInt(index + 1),
      })),
      text: { format: (_value, exact) => String(exact), unit: 'units', valueLabel: 'Amount' },
      tooltip: () => { tooltips++; return [{ title: 'Full country tooltip', rows: [] }]; },
      settings: { growth: 'share', labels: false, legend: true, breadcrumb: false, dimming: true },
    }));
    assert.equal(tooltips, 0, 'full tooltips are built only on hover/focus');
    const mounted = (html.match(/class="sunburst-name"/g) ?? []).length;
    assert.ok(mounted > 0 && mounted < 100, String(mounted));
    assert.equal((html.match(/class="country-flag-art sunburst-icon"/g) ?? []).length, mounted);
    assert.match(html, /aria-setsize="5000"/);
    assert.match(html, /COUNTRY-0: 1 units/, 'the complete spoken inventory is retained');
  } finally { await vite.close(); }
});

test('a focused legend row formats its full tooltip lazily and refreshes at the shown value', async () => {
  const state = { tip: null as { id: string; x: number; y: number; row: HTMLElement } | null,
    effects: [] as (() => void)[], rows: undefined as { items: { item: SunburstNode }[];
      children: (row: { item: SunburstNode }) => ReactElement } | undefined, shown: [] as unknown[] };
  const host = globalThis as typeof globalThis & { __sunburstTip?: typeof state };
  host.__sunburstTip = state;
  let formatted = 0;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'sunburst-tip-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/charts/useChartTokens.ts')) return "export const useChartTokens = () => ({ categories: ['#123'], text: '#fff', surface: '#000', fontSize: { body: 12 } });";
      if (path.endsWith('/charts/WindowedRows.tsx')) return 'export default props => { globalThis.__sunburstTip.rows = props; return null; };';
      if (path.endsWith('/charts/tooltip.ts')) return `export const tooltipStore = { get: () => null };
        export const hideTooltip = () => {}; export const fitTooltip = content => content;
        export const tooltipText = content => content.title;
        export const showTooltip = (_owner, content) => globalThis.__sunburstTip.shown.push(content);`;
      if (path.endsWith('/sunburst/Sunburst.tsx')) return source
        .replace(/const \[legendTip, setLegendTip\] = .*;/,
          'const [legendTip, setLegendTip] = [globalThis.__sunburstTip.tip, tip => { globalThis.__sunburstTip.tip = tip; }];')
        .replace(/\buseEffect\(/g, '(effect => globalThis.__sunburstTip.effects.push(effect))(');
    } }] });
  try {
    const { default: Sunburst } = await vite.ssrLoadModule('/ui/src/charts/sunburst/Sunburst.tsx');
    const render = (value: number) => {
      state.effects = [];
      renderToStaticMarkup(createElement(Sunburst, { label: 'Exact amounts', root: ROOT, active: true,
        fold: false, items: [item(['correction'], value, { exact: BigInt(value) })],
        text: { format: (_value: number, exact: bigint | null) => String(exact), unit: 'units', valueLabel: 'Amount' },
        tooltip: (node: SunburstNode) => { formatted++; return [{ title: node.label,
          rows: [{ label: 'Exact amount', value: String(node.exact) }], note: 'Recorded correction reason' }]; },
        settings: { growth: 'share', labels: false, legend: true, breadcrumb: false, dimming: true } }));
    };
    render(-1);
    assert.equal(formatted, 0);
    const row = state.rows!.children({ item: state.rows!.items[0].item });
    const entry = (row.props as { children: ReactElement<{ onFocus: (event: { currentTarget: HTMLElement }) => void }> }).children;
    entry.props.onFocus({ currentTarget: { getBoundingClientRect: () => ({ left: 5, bottom: 10 }) } as unknown as HTMLElement });
    render(-2);
    const effect = state.effects.find(effect => String(effect).includes('shownTip.current'));
    assert.ok(effect);
    effect();
    assert.equal(formatted, 1);
    assert.deepEqual(state.shown, [[{ title: 'CORRECTION', rows: [{ label: 'Exact amount', value: '-2' }], note: 'Recorded correction reason' }]]);
  } finally { await vite.close(); delete host.__sunburstTip; }
});

test('signed rows stay exact while only positive amounts receive area', () => {
  const root = groupTree([
    item(['positive'], 10, { exact: 10n }),
    item(['correction'], -20, { exact: -20n }),
  ], ROOT);
  const positive = root.children.find(node => node.key === 'positive')!;
  const correction = root.children.find(node => node.key === 'correction')!;
  assert.equal(root.exact, -10n);
  assert.equal(listed(correction), true);
  for (const growth of ['share', 'fill'] as const) {
    const arcs = arcTargets(root, root, growth);
    assert.equal(arcs.get(correction.id)?.shown, false);
    assert.equal(arcs.get(positive.id)?.shown, true);
    assert.equal(arcs.get(positive.id)?.a1, Math.PI * 2);
  }
  const text = { format: (_value: number, exact: bigint | null) => String(exact), unit: 'units', valueLabel: 'Amount' };
  assert.match(sunburstSummary(root, text), /CORRECTION: -20 units/);
  assert.equal(sliceTooltip(correction, root, text)[0].rows[0].value, '-20 units');
  assert.equal(sliceTooltip(positive, root, text)[0].rows.length, 1);
  const cancelled = groupTree([
    item(['group', 'positive'], 10, { exact: 10n }),
    item(['group', 'correction'], -10, { exact: -10n }),
  ], ROOT);
  const group = cancelled.children[0];
  assert.equal(group.exact, 0n);
  assert.equal(listed(group), true);
  const zoomed = arcTargets(cancelled, group, 'share');
  assert.equal(zoomed.get(group.children.find(node => node.key === 'positive')!.id)?.shown, true);
  assert.equal(zoomed.get(group.children.find(node => node.key === 'correction')!.id)?.shown, false);
});

test('nested corrections suppress net shares and Fill through positive groups and Other', () => {
  const parts = [
    item(['source', 'group', 'positive'], 20, { exact: 20n, final: 40 }),
    item(['source', 'group', 'correction'], -10, { exact: -10n, final: 10 }),
  ];
  const root = groupTree(parts, ROOT);
  const positive = root.children[0].children[0].children.find(node => node.key === 'positive')!;
  const text = { format: (_value: number, exact: bigint | null) => String(exact), unit: 'units', valueLabel: 'Amount' };
  assert.equal(root.value, 10);
  assert.equal(hasCorrections(root), true);
  assert.equal(sweepOf(root, 'fill'), Math.PI * 2);
  assert.deepEqual(sliceTooltip(positive, root, text)[0].rows.map(row => row.label), ['Amount']);
  assert.doesNotMatch(sunburstSummary(root, text), /%/);
  const future = groupTree([
    item(['source', 'group', 'positive'], 20, { exact: 20n, final: 40 }),
    item(['source', 'group', 'correction'], null, { present: false, final: -10 }),
  ], ROOT);
  assert.equal(future.value, 20);
  assert.equal(hasCorrections(future), true);
  assert.equal(sweepOf(future, 'fill'), Math.PI * 2);
  const collapsedFuture = groupTree([
    item(['source', 'group', 'positive'], 20, { exact: 20n, final: 40 }),
    item(['source', 'group', 'correction'], null, { present: false, final: -10 }),
  ], ROOT, [true, false, false], { value: 20, exact: 20n, final: 30 });
  assert.equal(hasCorrections(collapsedFuture), true);
  assert.equal(hasCorrections(collapsedFuture.children[0]), true);
  assert.equal(sweepOf(collapsedFuture, 'fill'), Math.PI * 2);
  assert.deepEqual(sliceTooltip(collapsedFuture.children[0], collapsedFuture, text)[0].rows.map(row => row.label), ['Amount']);
  const folded = foldTree(groupTree([
    item(['large'], 100, { exact: 100n }),
    ...parts,
    item(['second', 'positive'], 20, { exact: 20n, final: 40 }),
    item(['second', 'correction'], -10, { exact: -10n, final: 10 }),
  ], ROOT), 2);
  const other = folded.children.find(node => node.other)!;
  assert.equal(other.value, 20);
  assert.equal(hasCorrections(other), true);
  assert.equal(sweepOf(other, 'fill'), Math.PI * 2);
});

test('an incomplete full total keeps exact known geometry without percentage claims', () => {
  const root = groupTree([item(['known'], 30, { exact: 30n, final: 60, finalUnknown: true, unknown: { unread: 2 }, note: 'Two native amounts were not recorded.' }),
    item(['other'], 10, { exact: 10n, final: 20, finalUnknown: true })], ROOT);
  const known = root.children.find(node => node.key === 'known')!;
  const text = { format: (_value: number, exact: bigint | null) => String(exact), unit: 'units', valueLabel: 'Amount' };
  const partial = { ...text, shares: false };
  assert.equal(root.valueUnknown, false);
  assert.equal(root.finalUnknown, true);
  assert.equal(amountText(root, partial), '40');
  for (const growth of ['share', 'fill'] as const) {
    const arcs = arcTargets(root, root, growth);
    assert.equal(arcs.get(known.id)?.shown, true);
    assert.equal(arcs.get(known.id)?.a1, Math.PI * 1.5);
  }
  assert.deepEqual(sliceTooltip(known, root, partial)[0].rows.map(row => [row.label, row.value]),
    [['Amount', '30 units · partial']]);
  assert.equal(sliceTooltip(known, root, partial)[0].note, 'Two native amounts were not recorded.');
  assert.equal(spokenAmount(known, root, partial), '30 units, partial');
  assert.equal(sunburstSummary(root, partial), 'All: 40 units, partial; KNOWN: 30 units, partial; OTHER: 10 units');
  assert.ok(sliceTooltip(known, root, text)[0].rows.some(row => row.label === 'Of All' && row.value === '75%'));
  assert.match(spokenAmount(known, root, text), /75%/);
  assert.match(sunburstSummary(root, text), /75%/);
});

test('zooming into a zero-net group closes sibling subtrees and keeps ancestor motion', () => {
  const root = groupTree([
    item(['a', 'positive'], 10, { exact: 10n }),
    item(['a', 'correction'], -10, { exact: -10n }),
    item(['b', 'leaf'], 10, { exact: 10n }),
  ], ROOT);
  const a = root.children.find(node => node.key === 'a')!;
  const b = root.children.find(node => node.key === 'b')!;
  for (const growth of ['share', 'fill'] as const) {
    const targets = arcTargets(root, a, growth);
    assert.equal(targets.get(a.children.find(node => node.key === 'positive')!.id)!.shown, true);
    for (const outside of walk(b)) {
      const target = targets.get(outside.id)!;
      assert.equal(target.shown, false);
      assert.equal(target.a1, target.a0);
    }
    assert.deepEqual(targets.get(root.id), { a0: 0, a1: Math.PI * 2, ring: -1, shown: false });
  }
});



test('a collapsed exact remainder with huge fractional operands remains measured without unsafe geometry', () => {
  const denominator = 10n ** 400n;
  const root = groupTree([
    item(['group', 'positive'], 0, { exact: denominator + 1n, exactDenominator: denominator }),
    item(['group', 'correction'], 0, { exact: -denominator, exactDenominator: denominator }),
    item(['group', 'missing'], null, { note: 'An independently unmeasured part.' }),
  ], ROOT, [true, false]);
  const group = root.children[0];
  assert.deepEqual([group.exact, group.exactDenominator], [1n, denominator]);
  assert.equal(group.value, 0);
  assert.equal(Number.isFinite(group.value), true);
  assert.equal(listed(group), true);
  assert.equal(notMeasured(group), false);
  const text = { format: (_value: number, exact: bigint | null, units = 1n) => `${exact}/${units}`, unit: 'states', valueLabel: 'Amount' };
  assert.equal(sliceTooltip(group, root, text)[0].rows[0].value, `1/${denominator} states`);
});

test('fractions remain exact through hidden levels, grouping, folding and text formatting', () => {
  const fractions = [
    item(['country', 'a'], 1 / 3, { exact: 1n, exactDenominator: 3n, final: 1 / 3 }),
    item(['country', 'b'], 1 / 2, { exact: 1n, exactDenominator: 2n, final: 1 / 2 }),
    item(['country', 'c'], 1 / 6, { exact: 1n, exactDenominator: 6n, final: 1 / 6 }),
  ];
  const root = groupTree(fractions, ROOT);
  assert.equal(root.children[0].exact, 1n);
  assert.equal(root.children[0].exactDenominator, undefined);
  assert.equal(groupTree(fractions, ROOT, [true, false]).children[0].exact, 1n);
  const text = { format: (_value: number, exact: bigint | null, denominator = 1n) => `${exact}/${denominator}`,
    unit: 'states', valueLabel: 'Controlled' };
  const first = root.children[0].children.find(node => node.key === 'a')!;
  assert.equal(sliceTooltip(first, root, text)[0].rows[0].value, '1/3 states');
  const folded = foldTree(groupTree(fractions, ROOT), 1).children[0].children[0];
  assert.equal(folded.other, true);
  assert.equal(folded.exact, 1n);
  assert.equal(folded.exactDenominator, undefined);
});

test('signed fractions keep exact totals, correction rows, zeros and focused geometry together', () => {
  const parts = [
    item(['source', 'type', 'positive'], 2 / 3, { exact: 2n, exactDenominator: 3n, final: 4 / 3 }),
    item(['source', 'type', 'correction'], -1 / 3, { exact: -1n, exactDenominator: 3n, final: -2 / 3 }),
    item(['cancelled', 'positive'], 1 / 2, { exact: 1n, exactDenominator: 2n }),
    item(['cancelled', 'correction'], -1 / 2, { exact: -1n, exactDenominator: 2n }),
    { path: [key('cancelled'), { ...key('zero'), keepZero: true, colour: '#888888' }], value: 0, exact: 0n },
    item(['sibling', 'leaf'], 1 / 6, { exact: 1n, exactDenominator: 6n }),
  ];
  const root = groupTree(parts, ROOT, [], { value: 1 / 2, exact: 1n, exactDenominator: 2n, final: 5 / 6 });
  const source = root.children.find(node => node.key === 'source')!;
  const correction = source.children[0].children.find(node => node.key === 'correction')!;
  const cancelled = root.children.find(node => node.key === 'cancelled')!;
  const zero = cancelled.children.find(node => node.key === 'zero')!;
  const text = { format: (_value: number, exact: bigint | null, denominator = 1n) => `${exact}/${denominator}`,
    unit: 'states', valueLabel: 'Amount' };
  assert.deepEqual([root.exact, root.exactDenominator], [1n, 2n]);
  assert.deepEqual([source.exact, source.exactDenominator], [1n, 3n]);
  assert.equal(hasCorrections(root), true);
  assert.equal(sweepOf(root, 'fill'), Math.PI * 2);
  assert.deepEqual(sliceTooltip(correction, root, text)[0].rows.map(row => [row.label, row.value]), [['Amount', '-1/3 states']]);
  assert.equal(listed(correction), true);
  assert.equal(listed(cancelled), true);
  assert.equal(listed(zero), true);
  assert.equal(sliceTooltip(cancelled, root, text)[0].rows[0].value, '0/1 states');
  assert.equal(sliceColours(cancelled, ['#123456'], { light: '#ffffff', dark: '#000000' }).get(zero.id), '#888888');
  const targets = arcTargets(root, cancelled, 'fill');
  assert.equal(targets.get(cancelled.children.find(node => node.key === 'positive')!.id)!.shown, true);
  assert.equal(targets.get(cancelled.children.find(node => node.key === 'correction')!.id)!.shown, false);
  assert.equal(targets.get(zero.id)!.shown, false);
  for (const sibling of root.children.filter(node => node !== cancelled)) for (const node of walk(sibling)) {
    assert.equal(targets.get(node.id)!.shown, false);
    assert.equal(targets.get(node.id)!.a0, targets.get(node.id)!.a1);
  }
  const hidden = groupTree(parts, ROOT, [false, false, true]);
  assert.deepEqual([hidden.exact, hidden.exactDenominator], [1n, 2n]);
  const collapsed = groupTree(parts, ROOT, [true, false, false], { value: 1 / 2, exact: 1n, exactDenominator: 2n, final: 5 / 6 });
  const collapsedSource = collapsed.children.find(node => node.key === 'source')!;
  const collapsedCancellation = collapsed.children.find(node => node.key === 'cancelled')!;
  assert.equal(hasCorrections(collapsed), true);
  assert.equal(hasCorrections(collapsedSource), true);
  assert.equal(hasCorrections(collapsedCancellation), true);
  assert.equal(collapsedCancellation.children.length, 0);
  assert.equal(listed(collapsedCancellation), true);
  assert.deepEqual([collapsedCancellation.exact, collapsedCancellation.exactDenominator], [0n, undefined]);
  assert.equal(sweepOf(collapsed, 'fill'), Math.PI * 2);
  assert.deepEqual(sliceTooltip(collapsedSource, collapsed, text)[0].rows.map(row => row.label), ['Amount']);
  assert.doesNotMatch(sunburstSummary(collapsed, text), /%/);
  assert.equal(listed(groupTree([item(['ordinary-zero'], 0, { exact: 0n })], ROOT).children[0]), false);
  const folded = foldTree(groupTree(parts, ROOT), 1);
  const other = folded.children.find(node => node.other)!;
  assert.deepEqual([other.exact, other.exactDenominator], [1n, 2n]);
  assert.equal(hasCorrections(other), true);
  assert.equal(sweepOf(other, 'fill'), Math.PI * 2);
  assert.doesNotMatch(sunburstSummary(folded, text), /%/);
});

test('exact IC keeps missing-name, language and native-cause notes through grouping and tooltips', () => {
  const missingName = 'The state name was not recorded at this read.';
  const language = 'The recorded literal is unavailable in this language.';
  const cause = 'Native daily event; credit observed at the recorded hour.';
  const parts = [
    item(['building', 'state'], 2, { exact: 2n, note: missingName }),
    item(['building', 'state'], 3, { exact: 3n, note: missingName }),
    item(['building', 'state'], 4, { exact: 4n, note: language }),
    item(['building', 'state'], 1, { exact: 1n, note: cause }),
  ];
  const root = groupTree(parts, ROOT);
  const text = { format: (_value: number, exact: bigint | null) => String(exact), unit: 'IC', valueLabel: 'Spent' };
  for (const node of walk(root)) {
    assert.equal(node.valueUnknown, false);
    assert.equal(node.unmeasured, false);
    assert.equal(node.exact, 10n);
    assert.deepEqual(node.notes, [missingName, language, cause]);
    for (const tip of sliceTooltip(node, root, text)) {
      assert.equal(tip.rows[0].value, '10 IC');
      assert.equal(tip.note, `${missingName} ${language} ${cause}`);
    }
  }
  assert.deepEqual(groupTree(parts, ROOT, [false, true]).children[0].notes, [missingName, language, cause]);
  const folded = foldTree(groupTree([...parts, item(['large'], 20, { exact: 20n })], ROOT), 1);
  assert.equal(sliceTooltip(folded.children[0], folded, text)[0].note, `${missingName} ${language} ${cause}`);
});

test('incomplete attribution at a hidden level still reaches the whole and every shown ancestor', () => {
  const parts = [
    item(['new', 'type', 'design'], 1, { exact: 1n }),
    item(['other', 'unresolved'], 0, { exact: 0n, unknownLevels: [true, true, false] }),
  ];
  for (const shown of [[true, true, true], [false, false, true], [false, true, false], [true, false, false]]) {
    assert.equal(groupTree(parts, ROOT, shown).valueUnknown, true, JSON.stringify(shown));
  }
  const types = groupTree(parts, ROOT, [false, true, false]);
  assert.equal(types.children.find(node => node.key === 'type')!.valueUnknown, false);
  assert.equal(types.children.find(node => node.key === 'unresolved')!.valueUnknown, true);
  const local = groupTree([item(['new', 'design'], 1, { exact: 1n, localUnknownLevels: [true, false] })], ROOT, [false, true]);
  assert.equal(local.valueUnknown, false);
  assert.equal(local.children[0].valueUnknown, false);
});

test('an independent whole remains exact when content cannot completely attribute its groups', () => {
  const parts = [
    item(['land', 'type', 'design'], 10, { exact: 10n, unknownLevels: [true, true, false], note: 'Unresolved equipment could belong here.' }),
    item(['unresolved', 'token', 'other'], 5, { exact: 5n }),
  ];
  const total = { value: 15, exact: 15n, final: 15 };
  const root = groupTree(parts, ROOT, [true, true, true], total);
  const land = root.children[0];
  assert.equal(root.valueUnknown, false);
  assert.equal(root.exact, 15n);
  assert.equal(land.valueUnknown, true);
  assert.equal(land.children[0].valueUnknown, true);
  assert.equal(land.children[0].children[0].valueUnknown, false);
  assert.deepEqual(land.notes, ['Unresolved equipment could belong here.']);
  assert.equal(arcTargets(root, root, 'share').get(land.id)!.shown, false);
  assert.equal(arcTargets(root, root, 'share').get(land.children[0].id)!.shown, false);
  assert.equal(arcTargets(root, root, 'share').get(root.children[1].id)!.shown, true);
  const designs = groupTree(parts, ROOT, [false, false, true], total);
  assert.ok(designs.children.every(node => !node.valueUnknown));
  assert.equal(designs.exact, 15n);
  const before = [item(['land', 'type', 'design'], 10, { exact: 10n, final: 20, finalUnknownLevels: [true, true, false] })];
  const grouped = groupTree(before, ROOT, [true, true, true], { value: 10, exact: 10n, final: 20 });
  assert.equal(grouped.finalUnknown, false);
  assert.equal(grouped.children[0].finalUnknown, true);
  assert.equal(sweepOf(grouped.children[0], 'fill'), Math.PI * 2);
  const exactDesigns = groupTree(before, ROOT, [false, false, true], { value: 10, exact: 10n, final: 20 });
  assert.equal(exactDesigns.children[0].finalUnknown, false);
  assert.equal(sweepOf(exactDesigns.children[0], 'fill'), Math.PI);
});

test('an exact subtree stays drawable after zooming through an unknown whole', () => {
  const root = groupTree([
    item(['known', 'design'], 12, { exact: 12n }),
    item(['unknown', 'design'], null, { unknownValue: true }),
  ], ROOT, [], { value: null, exact: null, note: 'A read is missing.' });
  assert.equal(root.valueUnknown, true);
  assert.equal(root.exact, null);
  const known = root.children.find(child => child.key === 'known')!;
  assert.equal(arcTargets(root, known, 'share').get(known.children[0].id)!.shown, true);
});

test('an unknown total stays unknown beside exact parts, with no guessed shares or slices', () => {
  const root = groupTree([
    item(['land', 'type', 'design'], null, { unknownValue: true, note: 'Its delivery cost was not read.', final: 8 }),
    item(['air', 'type', 'design'], 20, { exact: 20n }),
  ], ROOT);
  const text = { format: (value: number) => String(value), unit: 'IC', valueLabel: 'Delivered value' };
  assert.equal(root.valueUnknown, true);
  assert.equal(root.exact, null);
  assert.equal(sweepOf(root, 'share'), 0);
  assert.ok([...arcTargets(root, root, 'share').values()].every(target => !target.shown));
  const land = root.children.find(node => node.key === 'land')!;
  const air = root.children.find(node => node.key === 'air')!;
  assert.equal(land.valueUnknown, true);
  assert.equal(listed(land), true);
  assert.equal(sliceTooltip(land, root, text)[0].rows[0].value, 'Not known');
  assert.equal(sliceTooltip(land, root, text)[0].note, 'Its delivery cost was not read.');
  assert.equal(air.valueUnknown, false);
  assert.equal(sliceTooltip(air, root, text)[0].rows.length, 1);
  assert.match(sunburstSummary(root, text), /^All: not known: Its delivery cost was not read\./);
  assert.equal(sweepOf(air, 'share'), Math.PI * 2);
  const flat = groupTree([
    item(['a'], 5, { exact: 5n }), item(['b'], null, { unknownValue: true }),
  ], ROOT, [false]);
  assert.equal(flat.valueUnknown, true);
  assert.equal(flat.exact, null);
});

test('Fill needs a known whole; an unknown denominator uses Share and survives folding', () => {
  const root = foldTree(groupTree(Array.from({ length: 10 }, (_, index) =>
    item([String(index)], index + 1, { exact: BigInt(index + 1), final: index + 2, finalUnknown: index === 0, unknownValue: index === 1 })), ROOT));
  assert.equal(root.finalUnknown, true);
  assert.equal(root.valueUnknown, true);
  const other = root.children.at(-1)!;
  assert.equal(other.finalUnknown, true);
  assert.equal(other.valueUnknown, true);
  assert.equal(other.exact, null);
  const early = groupTree([item(['a'], 10, { exact: 10n, final: 20, finalUnknown: true })], ROOT);
  assert.equal(early.valueUnknown, false);
  assert.equal(early.exact, 10n);
  assert.equal(sweepOf(early, 'fill'), Math.PI * 2);
  const absent = groupTree([item(['a'], null, { unknownValue: true, present: false, final: 20, finalUnknown: true })], ROOT);
  assert.equal(absent.valueUnknown, false);
  assert.equal(absent.finalUnknown, true);
});

test('a tree adds its items up and keeps missing measurement apart from zero', () => {
  const root = groupTree([
    item(['new', 'civ', 'ruhr'], 30, { final: 50 }),
    item(['new', 'civ', 'saxony'], 10, { final: 10, unknown: { gap: 1 } }),
    item(['new', 'fort', 'saar'], 0, { final: 5 }),
    item(['repair', 'civ', 'ruhr'], null, { note: 'Not measured' }),
  ], ROOT);
  assert.equal(root.value, 40);
  assert.equal(root.final, 65);
  assert.deepEqual(root.unknown, { gap: 1 });
  assert.deepEqual(root.children.map(child => child.key), ['new', 'repair']);
  const repair = root.children[1];
  assert.equal(repair.unmeasured, true);
  assert.equal(repair.value, 0);
  assert.deepEqual(repair.notes, ['Not measured']);
  assert.equal(root.unmeasured, false);
  const civ = root.children[0].children[0];
  assert.deepEqual(civ.children.map(child => child.key), ['ruhr', 'saxony']);
  assert.deepEqual(root.children[0].children.map(child => child.key), ['civ', 'fort']);
});

test('a level switched off joins its children, adding the same keys together', () => {
  const items = [item(['new', 'civ', 'ruhr'], 30), item(['new', 'fort', 'saar'], 5), item(['repair', 'civ', 'ruhr'], null, { note: 'n' }), item(['old', 'civ', 'ruhr'], 7)];
  const root = groupTree(items, ROOT, [false, true, false]);
  assert.deepEqual(root.children.map(child => [child.key, child.value, child.unmeasured]), [['civ', 37, false], ['fort', 5, false]]);
  assert.deepEqual(root.children[0].notes, ['n']);
  assert.deepEqual(root.children[0].children, []);
  const flat = groupTree(items, ROOT, [false, false, false]);
  assert.equal(flat.children.length, 0);
  assert.equal(flat.value, 42);
});

test('a correction to zero retains recorded peak palette slots and folded contributors', () => {
  const parts = (value: number) => [
    item(['source', 'peak'], value, { exact: BigInt(value), final: 0, weight: 20 }),
    item(['source', 'next'], 10, { exact: 10n }),
    item(['source', 'small'], 5, { exact: 5n }),
    item(['source', 'large'], 30, { exact: 30n }),
  ];
  const early = groupTree(parts(20), ROOT, [false, true]);
  const end = groupTree(parts(0), ROOT, [false, true]);
  const palette = ['#ff0000', '#00ff00', '#0000ff', '#ffff00'];
  const shades = { light: '#ffffff', dark: '#000000' };
  for (const tree of [early, end]) {
    assert.deepEqual(tree.children.map(node => node.key), ['large', 'peak', 'next', 'small']);
    assert.equal(tree.weight, 65);
    assert.equal(tree.final, 45);
    const colours = sliceColours(tree, palette, shades);
    assert.equal(colours.get(tree.children[1].id), palette[1]);
    assert.equal(colours.get(tree.children[2].id), palette[2]);
    const other = foldTree(tree, 2).children.find(node => node.other)!;
    assert.deepEqual(other.children.map(node => node.key), ['peak', 'next', 'small']);
    assert.equal(other.weight, 35);
    assert.equal(other.final, 15);
  }
  const peak = end.children.find(node => node.other)!.children[0];
  assert.equal(drawable(peak), true);
  assert.equal(peak.exact, 0n);
  assert.equal(arcTargets(end, end, 'share').get(peak.id)!.shown, false);
  assert.equal(end.exact, 45n);
  const negative = groupTree([item(['peak'], -5, { exact: -5n, final: -5, weight: 20 }),
    item(['positive'], 10, { exact: 10n })], ROOT);
  assert.equal(negative.children[0].key, 'peak');
  assert.equal(negative.children[0].exact, -5n);
  assert.equal(listed(negative.children[0]), true);
  assert.equal(negative.final, 5);
  assert.equal(hasCorrections(negative), true);
  assert.equal(sweepOf(negative, 'fill'), Math.PI * 2);
  const arcs = arcTargets(negative, negative, 'share');
  assert.equal(arcs.get(negative.children[0].id)!.shown, false);
  assert.equal(arcs.get(negative.children[1].id)!.a1 - arcs.get(negative.children[1].id)!.a0, Math.PI * 2);
});

test('mixed stable weights preserve exact fractions, truthful Fill and unknown folded attribution', () => {
  const parts = [
    item(['old', 'design'], 1 / 3, { exact: 1n, exactDenominator: 3n, final: 2 / 3,
      weight: 20, note: 'Recorded design note.', counted: ['same'] }),
    item(['new', 'design'], 1 / 6, { exact: 1n, exactDenominator: 6n, final: 1 / 3, counted: ['same'] }),
  ];
  const total = { value: 1 / 2, exact: 1n, exactDenominator: 2n, final: 1 };
  for (const inputs of [parts, [...parts].reverse()]) {
    const tree = groupTree(inputs, ROOT, [false, true], total);
    const design = tree.children[0];
    assert.equal(design.weight, 20 + 1 / 3);
    assert.equal(design.final, 1);
    assert.equal(design.exact, 1n);
    assert.equal(design.exactDenominator, 2n);
    assert.equal(design.count, 1);
    assert.deepEqual(design.notes, ['Recorded design note.']);
    assert.equal(sweepOf(design, 'fill'), Math.PI);
    assert.equal(tree.final, 1);
  }
  const unknownWhole = groupTree([item(['design'], 1 / 2,
    { exact: 1n, exactDenominator: 2n, weight: 20, final: null, finalUnknown: true })], ROOT);
  assert.equal(unknownWhole.weight, 20);
  assert.equal(unknownWhole.finalUnknown, true);
  assert.equal(unknownWhole.exact, 1n);
  assert.equal(sweepOf(unknownWhole, 'fill'), Math.PI * 2);
  const tree = foldTree(groupTree([...parts,
    item(['unknown', 'unread'], null, { weight: 4, final: null, finalUnknown: true, unknownValue: true,
      unknown: { missing: 2 }, note: 'The read is unavailable.' }),
  ], ROOT, [false, true], total), 1);
  const other = tree.children[0];
  assert.equal(other.weight, 24 + 1 / 3);
  assert.equal(other.valueUnknown, true);
  assert.equal(other.finalUnknown, true);
  assert.equal(other.exact, null);
  assert.deepEqual(other.unknown, { missing: 2 });
  assert.deepEqual(other.notes, ['Recorded design note.', 'The read is unavailable.']);
  assert.equal(tree.exact, 1n);
  assert.equal(tree.exactDenominator, 2n);
  assert.equal(tree.final, 1);
  assert.equal(tree.valueUnknown, false);
  assert.equal(sweepOf(other.children.find(node => node.key === 'unread')!, 'fill'), 0);
});

test('omitting weight retains signed final order, zero membership and independent whole semantics', () => {
  const tree = groupTree([
    item(['future'], 0, { present: false, final: 20 }),
    item(['current'], 10, { exact: 10n, final: 10 }),
    item(['negative'], -2, { exact: -2n, final: -5 }),
    item(['zero'], 0, { exact: 0n }),
    item(['unread'], null, { unknownValue: true, note: 'Not read.', unknown: { missing: 1 } }),
  ], ROOT, [], { value: 8, exact: 8n, final: 40 });
  assert.ok(walk(tree).every(node => node.weight === undefined));
  assert.deepEqual(tree.children.map(node => node.key), ['future', 'current', 'unread', 'zero', 'negative']);
  assert.deepEqual(tree.children.filter(drawable).map(node => node.key), ['future', 'current', 'negative']);
  const colours = sliceColours(tree, ['#ff0000', '#00ff00', '#0000ff'], { light: '#ffffff', dark: '#000000' });
  assert.equal(colours.get(tree.children.at(-1)!.id), '#0000ff');
  const other = foldTree(tree, 2).children.find(node => node.other)!;
  assert.deepEqual(other.children.map(node => node.key), ['current', 'negative']);
  assert.equal(other.weight, undefined);
  assert.equal(other.exact, 8n);
  assert.equal(other.final, 5);
  assert.equal(tree.exact, 8n);
  assert.equal(tree.final, 40);
  assert.equal(tree.valueUnknown, false);
  assert.deepEqual(tree.unknown, { missing: 1 });
});

test('more slices than colours fold their smallest into Other, the same ones all playback long', () => {
  const items = Array.from({ length: 11 }, (_, index) => item([`t${index}`], index === 0 ? 0 : 100 - index, { final: 200 - index }));
  const root = foldTree(groupTree(items, ROOT), 8, 'Other');
  assert.equal(root.children.length, 8);
  const other = root.children.at(-1)!;
  assert.equal(other.other, true);
  assert.equal(root.children[0].key, 't0');
  assert.deepEqual(other.children.map(child => child.key), ['t7', 't8', 't9', 't10']);
  assert.equal(other.value, 93 + 92 + 91 + 90);
  assert.equal(other.children[0].depth, 2);
  assert.equal(other.children[0].parent, other);
  assert.equal(findNode(root, other.children[0].id), other.children[0]);
  assert.equal(findNode(root, `${other.children[0].id}\u0000gone`), other.children[0]);
  assert.equal(findNode(root, 'nowhere'), root);
  assert.deepEqual(pathTo(other.children[0]).map(node => node.key), ['all', '\u0001other', 't7']);
});

test('the zoomed slice fills the circle and its children share it; Fill sweeps toward the whole', () => {
  const root = groupTree([item(['a', 'x'], 30, { final: 60 }), item(['a', 'y'], 10, { final: 20 }), item(['b', 'z'], 40, { final: 40 })], ROOT);
  const all = arcTargets(root, root, 'share');
  const [a, b] = root.children;
  assert.ok(Math.abs(all.get(a.id)!.a1 - Math.PI) < 1e-9);
  assert.equal(all.get(a.id)!.ring, 1);
  assert.equal(all.get(a.children[0].id)!.ring, 2);
  assert.equal(all.get(a.children[0].id)!.shown, true);
  const zoomed = arcTargets(root, a, 'share');
  assert.ok(Math.abs(zoomed.get(a.children[0].id)!.a1 - Math.PI * 1.5) < 1e-9);
  assert.equal(zoomed.get(a.children[0].id)!.ring, 1);
  assert.equal(zoomed.get(b.id)!.shown, false);
  assert.equal(zoomed.get(a.id)!.shown, false);
  assert.ok(Math.abs(sweepOf(root, 'fill') - Math.PI * 2 * 80 / 120) < 1e-9);
  const filled = arcTargets(root, root, 'fill');
  assert.ok(Math.abs(filled.get(b.id)!.a1 - sweepOf(root, 'fill')) < 1e-9);
  const deep = groupTree([item(['a', 'b', 'c'], 1)], ROOT);
  assert.equal(arcTargets(deep, deep, 'share').get(walk(deep).at(-1)!.id)!.shown, false);
});

test('5,000 categories make one Other level', () => {
  const items = Array.from({ length: 5_000 }, (_, index) => item(['parent', `t${index}`], 5_000 - index, { exact: BigInt(5_000 - index), counted: [`p${index}`] }));
  const root = foldTree(groupTree(items, ROOT));
  const parent = root.children[0];
  const other = parent.children.at(-1)!;
  assert.equal(parent.children.length, 8);
  assert.equal(other.other, true);
  assert.equal(other.children.length, 4_993);
  assert.equal(walk(root).filter(node => node.other).length, 1);
  assert.equal(Math.max(...walk(root).map(node => node.depth)), 3);
  assert.ok(other.children.every(child => !child.other && child.parent === other));
  assert.equal(root.count, 5_000);
  assert.equal(root.exact, 12_502_500n);
});

test('Other opens and folds its parts only on the zoom path, which survives playback', () => {
  const items = Array.from({ length: 30 }, (_, index) => item([`t${index}`, 'place'], 30 - index, { exact: BigInt(30 - index) }));
  const root = foldTree(groupTree(items, ROOT));
  const other = root.children.at(-1)!;
  const hidden = other.children[0];
  assert.equal(arcTargets(root, root, 'share').get(hidden.id)!.shown, false);
  assert.equal(sliceColours(root, Array(8).fill('#ff0000'), { light: '#ffffff', dark: '#000000' }).has(hidden.id), false);
  const focus = foldView(root, [other.id]);
  assert.equal(focus, other);
  assert.equal(focus.children.length, 8);
  const nested = focus.children.at(-1)!;
  assert.equal(nested.other, true);
  assert.ok(nested.children.every(child => !child.other));
  const rings = keyRings(root, arcTargets(root, focus, 'share'));
  assert.equal(rings[0].length, 8);
  assert.equal(arcTargets(root, focus, 'share').get(hidden.id)!.shown, true);
  assert.ok(rings[1].every(node => node.parent !== nested));
  assert.equal(focus.value, 276);
  assert.equal(focus.exact, 276n);
  const deeper = foldView(root, [other.id, nested.id]);
  const part = deeper.children[0];
  const path = pathTo(part).slice(1).map(node => node.id);
  const rebuilt = foldTree(groupTree(items.map(item => ({ ...item, value: item.value! / 2, final: item.value })), ROOT));
  const restored = foldView(rebuilt, path);
  assert.equal(restored.id, part.id);
  assert.deepEqual(pathTo(restored).slice(1).map(node => node.id), path);
  assert.equal(walk(rebuilt).filter(node => node.other).length, walk(root).filter(node => node.other).length);
  const back = foldTree(groupTree(items, ROOT));
  foldView(back, [other.id]);
  assert.equal(walk(back).filter(node => node.other).length, 2);
});

test('the innermost ring takes the palette in order and outer rings shade their parent', () => {
  const root = groupTree([item(['a', 'x'], 3), item(['a', 'y'], 2), item(['b', 'z'], 1)], ROOT);
  const colours = sliceColours(root, ['#ff0000', '#00ff00'], { light: '#ffffff', dark: '#000000' });
  const [a, b] = root.children;
  assert.equal(colours.get(a.id), '#ff0000');
  assert.equal(colours.get(b.id), '#00ff00');
  assert.equal(colours.get(a.children[0].id), '#ff0000');
  assert.notEqual(colours.get(a.children[1].id), '#ff0000');
  const zoomed = sliceColours(a, ['#0000ff', '#00ff00'], { light: '#ffffff', dark: '#000000' });
  assert.equal(zoomed.get(a.children[1].id), '#00ff00');
  assert.equal(zoomed.has(b.id), false);
});

test('a point finds the centre or the slice drawn under it, and labels fit only where there is room', () => {
  const g = sunGeometry(200, 200);
  assert.equal(g.cx, 100);
  assert.ok(g.hole < g.radius);
  const slices = [
    { id: 'right', a0: 0, a1: Math.PI, inner: g.hole, outer: g.hole + g.ring, alpha: 1 },
    { id: 'fading', a0: Math.PI, a1: Math.PI * 2, inner: g.hole, outer: g.hole + g.ring, alpha: 0.2 },
  ];
  assert.equal(sliceAt(slices, g, 100, 100), 'centre');
  const middle = g.hole + g.ring / 2;
  assert.equal(sliceAt(slices, g, 100 + middle, 100), 'right');
  assert.equal(sliceAt(slices, g, 100 - middle, 100), null);
  assert.equal(sliceAt(slices, g, 199.9, 199.9), null);
  assert.equal(labelFits({ a0: 0, a1: Math.PI / 2, inner: 40, outer: 70 }, 40, 11), true);
  assert.equal(labelFits({ a0: 0, a1: 0.1, inner: 40, outer: 70 }, 40, 11), false);
  assert.equal(labelFits({ a0: 0, a1: Math.PI, inner: 40, outer: 50 }, 20, 11), false);
  assert.equal(labelFits({ a0: 0, a1: Math.PI * 2 / 3, inner: 95.2, outer: 140 }, 180, 11), false);
  assert.equal(labelFits({ a0: 0, a1: Math.PI * 2 / 3, inner: 95.2, outer: 140 }, 60, 11), true);
});

test('the keys go round a ring, out to a child and back in to its parent', () => {
  const root = groupTree([item(['a', 'x'], 3), item(['a', 'y'], 2), item(['b', 'z'], 4)], ROOT);
  const rings = keyRings(root, arcTargets(root, root, 'share'));
  assert.deepEqual(rings.map(ring => ring.map(node => node.key)), [['a', 'b'], ['x', 'y', 'z']]);
  const a = keyedSlice('ArrowRight', null, rings)!;
  assert.equal(a.key, 'a');
  assert.equal(keyedSlice('ArrowRight', a.id, rings)!.key, 'b');
  assert.equal(keyedSlice('ArrowLeft', a.id, rings)!.key, 'b');
  const x = keyedSlice('ArrowDown', a.id, rings)!;
  assert.equal(x.key, 'x');
  assert.equal(keyedSlice('End', x.id, rings)!.key, 'z');
  assert.equal(keyedSlice('ArrowUp', x.id, rings), a);
  assert.equal(keyedSlice('ArrowUp', a.id, rings), a);
  assert.equal(keyedSlice('ArrowDown', 'gone', rings)!.key, 'a');
});

test('a slice tooltip gives its exact amount, shares, count and what is not counted, fullest first', () => {
  const root = groupTree([
    item(['a', 'x'], 30, { counted: ['p1', 'p2'], unknown: { gap: 2, late: 1 } }),
    item(['a', 'y'], 10, { counted: ['p3'] }),
    item(['r', 'z'], null, { note: 'Not measured here.', counted: ['p4', 'p5', 'p6', 'p7'] }),
  ], ROOT);
  const text = { format: (value: number) => value.toFixed(0), unit: 'IC', valueLabel: 'Spent', countLabel: 'Projects' };
  const [a, r] = root.children;
  const x = a.children[0];
  const [full, compact] = sliceTooltip(x, root, text);
  assert.equal(full.title, 'A › X');
  assert.deepEqual(full.rows.map(row => [row.label, row.value]), [
    ['Spent', '30 IC · partial'], ['Of A', '75%'], ['Of All', '75%'], ['Projects', '2'],
  ]);
  assert.deepEqual(compact.rows.map(row => row.label), ['Spent', 'Of A', 'Of All', 'Projects']);
  const [unmeasured] = sliceTooltip(r, root, text);
  assert.deepEqual(unmeasured.rows.map(row => [row.label, row.value]), [['Spent', 'Not measured'], ['Projects', '4']]);
  assert.equal(unmeasured.note, 'Not measured here.');
  assert.equal(sunburstSummary(root, text), 'All: 40 IC, partial; A: 40 IC, 100%, partial; R: not measured');
  assert.equal(sunburstSummary(r, text), 'R: not measured; Z: not measured');
});


test('a part with nothing counted yet but parts not counted stays listed, with its uncertainty', () => {
  const root = groupTree([item(['a'], 50), item(['gap'], 0, { final: 0, unknown: { notRecorded: 1 } })], ROOT);
  const gap = root.children.find(child => child.key === 'gap')!;
  assert.equal(gap.unmeasured, false);
  assert.equal(listed(gap), true);
  const text = { format: (value: number) => String(value), unit: 'IC', valueLabel: 'Spent' };
  assert.equal(sunburstSummary(root, text), 'All: 50 IC, partial; A: 50 IC, 100%; GAP: 0 IC, partial');
  assert.equal(listed(groupTree([item(['a'], 0, { final: 0 })], ROOT).children[0]), false);
});

test('an item not yet present adds only its whole amount: order, colours and Fill are the end\'s', () => {
  const at = (bPresent: boolean, aValue: number, bValue: number | null) => groupTree([
    item(['a'], aValue, { final: 100 }),
    item(['b'], bValue, { final: 900, present: bPresent }),
  ], ROOT);
  const early = at(false, 100, null);
  assert.equal(early.value, 100);
  assert.equal(early.final, 1000);
  assert.ok(Math.abs(sweepOf(early, 'fill') - Math.PI * 2 * 0.1) < 1e-9);
  const [b] = early.children;
  assert.equal(b.key, 'b');
  assert.equal(b.present, false);
  assert.equal(listed(b), false);
  const late = at(true, 100, 900);
  assert.deepEqual(early.children.map(child => child.key), late.children.map(child => child.key));
  const palette = ['#ff0000', '#00ff00'];
  const shades = { light: '#ffffff', dark: '#000000' };
  const a = (tree: typeof early) => sliceColours(tree, palette, shades).get(tree.children.find(child => child.key === 'a')!.id);
  assert.equal(a(early), a(late));
});

test('equal whole amounts keep one order by data key, whatever the amounts so far', () => {
  const tree = (x: number, y: number) => groupTree([item(['y'], y, { final: 50 }), item(['x'], x, { final: 50 })], ROOT);
  assert.deepEqual(tree(10, 40).children.map(child => child.key), ['x', 'y']);
  assert.deepEqual(tree(40, 10).children.map(child => child.key), ['x', 'y']);
  const many = (bump: number) => foldTree(groupTree(Array.from({ length: 10 }, (_, index) =>
    item([`k${index}`], index === 9 ? bump : 1, { final: 5 })), ROOT), 8);
  assert.deepEqual(many(0).children.at(-1)!.children.map(child => child.key), many(4).children.at(-1)!.children.map(child => child.key));
});

test('exact amounts add up without rounding through a number', () => {
  const root = groupTree([item(['a'], 9007199254741499, { exact: 9007199254741499n }), item(['b'], 2, { exact: 2n })], ROOT);
  assert.equal(root.exact, 9007199254741501n);
  assert.equal(groupTree([item(['a'], 1, { exact: 1n }), item(['b'], 2)], ROOT).exact, null);
  assert.equal(groupTree([item(['r'], null, { note: 'n' })], ROOT).children[0].exact, null);
});


test('folding goes by whole amounts, so a type that is repaired first and built later folds the same all along', () => {
  const tree = (built: boolean) => foldTree(groupTree([
    ...Array.from({ length: 8 }, (_, index) => item([`t${index}`], 10 + index, { final: 100 + index })),
    item(['z'], null, { note: 'n' }),
    item(['z'], built ? 50 : null, { final: 50, present: built }),
  ], ROOT), 8);
  const other = (root: ReturnType<typeof tree>) => root.children.find(child => child.other)!.children.map(child => child.key);
  assert.deepEqual(other(tree(false)), other(tree(true)));
  assert.ok(other(tree(false)).includes('z'));
});

test('parts that are never drawn take no palette slot', () => {
  const root = groupTree([item(['r'], null, { note: 'n' }), item(['u'], 0, { final: 0, unknown: { gap: 1 } }), item(['x'], 5), item(['y'], 3)], ROOT);
  const colours = sliceColours(root, ['#ff0000', '#00ff00'], { light: '#ffffff', dark: '#000000' });
  const id = (key: string) => root.children.find(child => child.key === key)!.id;
  assert.equal(colours.get(id('x')), '#ff0000');
  assert.equal(colours.get(id('y')), '#00ff00');
  assert.equal(colours.has(id('r')), false);
  assert.equal(colours.has(id('u')), false);
});

test('Other is not measured when only an unmeasured part of it is present', () => {
  const items = [
    ...Array.from({ length: 9 }, (_, index) => item([`t${index}`], null, { final: 100 - index, present: false })),
    item(['t8'], null, { note: 'Repairs are not measured.' }),
  ];
  const root = foldTree(groupTree(items, ROOT), 8);
  const other = root.children.find(child => child.other)!;
  assert.equal(other.present, true);
  assert.equal(other.unmeasured, true);
  assert.equal(listed(other), true);
  assert.equal(other.exact, null);
  assert.deepEqual(other.notes, ['Repairs are not measured.']);
  const measured = foldTree(groupTree([...items, item(['t7'], 5, { exact: 5n, final: 0 })], ROOT), 8);
  const withAmount = measured.children.find(child => child.other)!;
  assert.equal(withAmount.unmeasured, false);
  assert.equal(withAmount.exact, 5n);
});

test('a project in two merged categories counts once', () => {
  const root = groupTree([
    item(['repair', 'civ'], null, { note: 'n', counted: ['9'] }),
    item(['new', 'civ'], 40, { counted: ['9', '10'] }),
  ], ROOT, [false, true]);
  assert.equal(root.children[0].count, 2);
  assert.equal(root.count, 2);
});

test('a merged category with unmeasured parts and nothing counted stays listed', () => {
  const root = groupTree([
    item(['repair', 'civ'], null, { note: 'Repairs are not measured.' }),
    item(['new', 'civ'], 0),
  ], ROOT, [false, true]);
  const civ = root.children[0];
  assert.equal(civ.unmeasured, false);
  assert.equal(civ.partlyUnmeasured, true);
  assert.equal(listed(civ), true);
  const text = { format: (value: number) => String(value), unit: 'IC', valueLabel: 'Spent' };
  assert.equal(sunburstSummary(root, text), 'All: not measured; CIV: not measured');
  const [tip] = sliceTooltip(civ, root, text);
  assert.deepEqual(tip.rows[0], { label: 'Spent', value: 'Not measured', tone: 'muted' });
});

test('grouping and folding retain dated country flags without marking aggregates or changing exact notes', () => {
  const statuses = [{ flagPending: true }, { flagReason: 'Recorded content lacks this flag' }, { flag: 'dated.png' }];
  const tree = foldTree(groupTree(statuses.map((countryFlag, index) => ({
    path: [{ id: String(index), label: `Country ${index}`, countryFlag }], value: index + 1, exact: BigInt(index + 1),
    note: 'Name content not available',
  })), ROOT), 2);
  const other = tree.children.find(node => node.other)!;
  assert.equal(tree.countryFlag, undefined);
  assert.equal(other.countryFlag, undefined);
  for (const node of walk(tree).filter(node => node.countryFlag !== undefined)) {
    assert.equal(node.countryFlag, statuses[Number(node.key)]);
    const [tip] = sliceTooltip(node, tree, { format: (_value, exact) => String(exact), unit: 'units', valueLabel: 'Amount' });
    assert.equal(tip.countryFlag, node.countryFlag);
    assert.equal(tip.note, 'Name content not available');
    assert.equal(node.valueUnknown, false);
    assert.equal(node.exact, BigInt(Number(node.key) + 1));
  }
  assert.equal(walk(tree).filter(node => node.countryFlag !== undefined).length, 3);
});

test('missing item artwork in a sunburst legend renders a named ArtFallback while text-only levels omit placeholders', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'sunburst-art-theme', enforce: 'pre', transform(_source, id) {
      if (id.replaceAll('\\', '/').endsWith('/charts/useChartTokens.ts')) return "export const useChartTokens = () => ({ categories: ['#123'], text: '#fff', surface: '#000', fontSize: { small: 10, body: 12 } });";
    } }] });
  try {
    const { default: Sunburst } = await vite.ssrLoadModule('/ui/src/charts/sunburst/Sunburst.tsx') as typeof import('./Sunburst.tsx');
    const withMissing = renderToStaticMarkup(createElement(Sunburst, {
      label: 'Buildings', root: ROOT, active: true, fold: false,
      items: [{ path: [{ id: 'reactor', label: 'Nuclear Reactor', icon: null }], value: 10, exact: 10n }],
      text: { format: (_value, exact) => String(exact), unit: 'IC', valueLabel: 'Spent' },
      settings: { growth: 'share', labels: false, legend: true, breadcrumb: false, dimming: true },
    }));
    assert.match(withMissing, /class="art-fallback sunburst-icon"/);
    assert.match(withMissing, /aria-label="Nuclear Reactor — Art not in the current installation\."/);

    const textOnly = renderToStaticMarkup(createElement(Sunburst, {
      label: 'Categories', root: ROOT, active: true, fold: false,
      items: [{ path: [{ id: 'cat', label: 'Industry' }], value: 10, exact: 10n }],
      text: { format: (_value, exact) => String(exact), unit: 'IC', valueLabel: 'Spent' },
      settings: { growth: 'share', labels: false, legend: true, breadcrumb: false, dimming: true },
    }));
    assert.doesNotMatch(textOnly, /art-fallback/);
  } finally { await vite.close(); }
});
