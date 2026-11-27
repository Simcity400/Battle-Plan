import assert from 'node:assert/strict';
import test from 'node:test';
import { allWidgetPages, costPages, enumerateWidgetPlacements, FIXTURE, HEAVIEST, heaviestKinds, LEGIBLE, LEGIBLE_ROWS, REBUILT, REBUILT_ALONE, REBUILT_KINDS, SCREEN, selectHeaviestKinds, VIEWS, legible, pack, screenMetrics, tallViewportHeight, tightest } from './allWidgets.mjs';

const overlaps = (a, b) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
const resting = (item, all) => item.y === 0 || all.some(other => other !== item && other.y + other.h === item.y && other.x < item.x + item.w && item.x < other.x + other.w);

test('packing puts each item where it sits highest, never over another or past the grid', () => {
  const sizes = [{ w: 12, h: 3 }, { w: 8, h: 4 }, { w: 4, h: 2 }, { w: 4, h: 2 }, { w: 6, h: 1 }, { w: 6, h: 5 }];
  const placed = pack(sizes, 12).map((at, index) => ({ ...at, ...sizes[index] }));
  assert.deepEqual(placed.map(({ x, y }) => [x, y]), [[0, 0], [0, 3], [8, 3], [8, 5], [0, 7], [6, 7]]);
  for (const [index, item] of placed.entries()) {
    assert.ok(item.x + item.w <= 12);
    for (const other of placed.slice(index + 1)) assert.ok(!overlaps(item, other), `${JSON.stringify(item)} overlaps ${JSON.stringify(other)}`);
  }
  assert.throws(() => pack([{ w: 13, h: 1 }], 12), /does not fit/);
});

const assertPacked = (placed, cols, rows = Infinity) => {
  for (const [index, item] of placed.entries()) {
    assert.ok(item.x >= 0 && item.y >= 0 && item.x + item.w <= cols && item.y + item.h <= rows,
      `${JSON.stringify(item)} is outside ${cols}x${rows}`);
    for (const other of placed.slice(index + 1)) assert.ok(!overlaps(item, other), `${JSON.stringify(item)} overlaps ${JSON.stringify(other)}`);
  }
};

test('fixture packing stays at its expected placements and within the grid', () => {
  const sizes = [{ w: 4, h: 1 }, { w: 8, h: 2 }, { w: 4, h: 3 }, { w: 6, h: 2 }, { w: 6, h: 2 }, { w: 5, h: 4 }];
  const tight = tightest(sizes, 12);
  assert.deepEqual(tight.sorted.map(({ w, h }) => [w, h]), [[8, 2], [6, 2], [6, 2], [5, 4], [4, 3], [4, 1]]);
  assert.deepEqual(tight.places.map(({ x, y }) => [x, y]), [[0, 0], [0, 2], [6, 2], [0, 4], [5, 4], [5, 7]]);
  assertPacked(tight.places.map((at, index) => ({ ...at, ...tight.sorted[index] })), 12);

  const shown = legible([{ w: 12, h: 3 }, { w: 6, h: 2 }, { w: 6, h: 2 }, { w: 4, h: 1 }, { w: 8, h: 2 }], 12, 3);
  assert.equal(shown.inView, 2);
  assert.deepEqual(shown.sorted.slice(0, 2).map(({ w, h }) => [w, h]), [[4, 1], [6, 2]]);
  assert.deepEqual(shown.places.map(({ x, y }) => [x, y]), [[0, 0], [4, 0], [0, 2], [0, 5], [0, 7]]);
  const placed = shown.places.map((at, index) => ({ ...at, ...shown.sorted[index] }));
  assertPacked(placed, 12);
  for (const item of placed.slice(0, shown.inView)) assert.ok(item.y + item.h <= 3, `${JSON.stringify(item)} in view`);
  for (const item of placed.slice(shown.inView)) assert.ok(item.y + item.h > 3, `${JSON.stringify(item)} beyond view`);
});

test('the screen view shows a page on exactly the screen\'s device pixels, at the largest pixel ratio that holds it', () => {
  for (const height of [1, 2160, 2161, 32_780]) {
    const metrics = screenMetrics(height);
    assert.ok(metrics.height >= height, `${height} CSS px fit`);
    assert.deepEqual([metrics.width * metrics.deviceScaleFactor, metrics.height * metrics.deviceScaleFactor].map(Math.round), [SCREEN.width, SCREEN.height]);
    assert.equal(metrics.width * 9, metrics.height * 16);
    assert.ok(metrics.height - 9 < height, `${height}: ${metrics.height} is the smallest`);
  }
  assert.deepEqual(screenMetrics(2160), { width: 3840, height: 2160, deviceScaleFactor: 1 });
  assert.deepEqual(screenMetrics(32_780), { width: 58_288, height: 32_787, deviceScaleFactor: 240 / 3643 });
  assert.deepEqual(screenMetrics(100, { width: 1000, height: 1000 }), { width: 100, height: 100, deviceScaleFactor: 10 });
});

test('the tall view includes the shell header when growing past the grid height', () => {
  assert.equal(tallViewportHeight(1200, 900), 1282);
  assert.equal(tallViewportHeight(700, 900), 900);
});

test('the fixture pages hold every widget the app offers once, at its minimum or default size, as the app\'s own storage accepts and restores them', async () => {
  const [{ widgetMetas }, storage, pages] = await Promise.all([import('../../ui/src/widgets/definitions.ts'),
    import('../../ui/src/dashboard/storage.ts'), import('../../ui/src/dashboard/pages.ts')]);
  const { entries, pages: built } = await allWidgetPages();
  const saved = new Map(entries);
  const read = { getItem: key => saved.get(key) ?? null, setItem: () => { throw new Error('read only'); } };
  const listed = pages.loadPages(read);
  assert.deepEqual(listed.map(entry => entry.id), [...pages.PAGES.map(entry => entry.id), built[FIXTURE].id, built[LEGIBLE].id, built[REBUILT].id, built[REBUILT_ALONE].id], 'the built-in pages stay');
  for (const [view, { page }] of Object.entries(VIEWS)) if (view !== HEAVIEST) assert.ok(built[page], `${view} plays a page the fixture builds`);
  for (const [label, sizeOf] of [[FIXTURE, meta => meta.minSize], [LEGIBLE, meta => meta.defaultSize]]) {
    const page = built[label];
    assert.equal(listed.find(entry => entry.id === page.id).label, label);
    assert.deepEqual(page.widgets.map(widget => widget.kind).sort(), widgetMetas.map(meta => meta.kind).sort(), `${label}: every kind, each once`);
    const { library, warning } = storage.loadLayouts(read, page.id);
    assert.equal(warning, null, `${label} restores whole`);
    const template = library.layouts.find(entry => entry.id === library.activeId).template;
    assert.deepEqual(template.widgets.map(widget => widget.id), page.widgets.map(widget => widget.id));
    const lg = template.layouts.lg;
    for (const widget of template.widgets) {
      const meta = widgetMetas.find(item => item.kind === widget.kind);
      const item = lg.find(entry => entry.i === widget.id);
      assert.deepEqual([item.w, item.h], [sizeOf(meta).w, sizeOf(meta).h], `${label}: ${widget.kind} at its size`);
      assert.equal(widget.options, undefined, `${widget.kind} at its default settings`);
    }
    for (const [index, item] of lg.entries()) {
      assert.ok(resting(item, lg), `${label}: ${item.i} stays where the grid's compaction leaves it`);
      for (const other of lg.slice(index + 1)) assert.ok(!overlaps(item, other), `${item.i} overlaps ${other.i}`);
    }
    assert.equal(page.rows, Math.max(...lg.map(item => item.y + item.h)));
  }
  const rebuilt = built[REBUILT], rebuiltLayout = storage.loadLayouts(read, rebuilt.id);
  assert.equal(rebuiltLayout.warning, null, 'the rebuilt page restores whole');
  assert.equal(rebuilt.widgets.length, widgetMetas.length);
  assert.deepEqual([...new Set(rebuilt.widgets.map(widget => widget.kind))].sort(), [...REBUILT_KINDS].sort());
  assert.equal(new Set(rebuilt.widgets.map(widget => widget.id)).size, rebuilt.widgets.length, 'each copy its own widget');
  const rebuiltFit = screenMetrics(rebuilt.rows * 32);
  assert.deepEqual([rebuiltFit.width * rebuiltFit.deviceScaleFactor, rebuiltFit.height * rebuiltFit.deviceScaleFactor].map(Math.round), [SCREEN.width, SCREEN.height]);
  const alone = built[REBUILT_ALONE];
  assert.equal(storage.loadLayouts(read, alone.id).warning, null, 'the page of each rebuilt widget alone restores whole');
  assert.deepEqual(alone.widgets.map(widget => widget.kind).sort(), [...REBUILT_KINDS].sort(), 'each rebuilt widget once');
  const minimum = built[FIXTURE];
  assert.equal(minimum.inView, minimum.widgets.length);
  assert.ok(minimum.rows <= Math.ceil(widgetMetas.reduce((sum, meta) => sum + meta.minSize.w * meta.minSize.h, 0) / 12) + 8, `${minimum.rows} rows`);
  const fitted = screenMetrics(minimum.rows * 32);
  assert.deepEqual([fitted.width * fitted.deviceScaleFactor, fitted.height * fitted.deviceScaleFactor].map(Math.round), [SCREEN.width, SCREEN.height], 'minimum page fits the 4K device pixels');
  const page = built[LEGIBLE];
  const lg = storage.loadLayouts(read, page.id).library.layouts[0].template.layouts.lg;
  const at = id => lg.find(item => item.i === id);
  assert.ok(page.inView > 0);
  for (const widget of page.widgets.slice(0, page.inView)) assert.ok(at(widget.id).y + at(widget.id).h <= LEGIBLE_ROWS, `${widget.kind} whole in view`);
  for (const widget of page.widgets.slice(page.inView)) assert.ok(at(widget.id).y + at(widget.id).h > LEGIBLE_ROWS, `${widget.kind} beyond view`);
});

test('costPages builds an empty page and one default-size page per requested kind, and heaviest builds the top N', async () => {
  const [{ widgetMetas }, storage, pages] = await Promise.all([import('../../ui/src/widgets/definitions.ts'),
    import('../../ui/src/dashboard/storage.ts'), import('../../ui/src/dashboard/pages.ts')]);
  const built = await costPages(['scoreboard', 'research']);
  const saved = new Map(built.entries);
  const read = { getItem: key => saved.get(key) ?? null, setItem: () => { throw new Error('read only'); } };
  const listed = pages.loadPages(read);
  assert.deepEqual(listed.map(p => p.label), ['CostEmpty', 'Cost:scoreboard', 'Cost:research']);
  const emptyLayout = storage.loadLayouts(read, built.empty.id);
  assert.equal(emptyLayout.warning, null);
  assert.equal(emptyLayout.library.layouts[0].template.widgets.length, 0);
  const scoreLayout = storage.loadLayouts(read, built.kinds[0].id);
  assert.equal(scoreLayout.warning, null);
  assert.equal(scoreLayout.library.layouts[0].template.widgets[0].kind, 'scoreboard');
  const meta = widgetMetas.find(m => m.kind === 'scoreboard');
  const box = scoreLayout.library.layouts[0].template.layouts.lg[0];
  assert.deepEqual([box.w, box.h], [meta.defaultSize.w, meta.defaultSize.h]);
  await assert.rejects(() => costPages(['no-such-widget']), /no-such-widget/);

  const costs = { widgets: [
    { kind: 'research', costScoreMs: 5, mainDelta: { p50: 1 }, worker: { mean: 0 }, outcome: 'content' },
    { kind: 'scoreboard', costScoreMs: 12, mainDelta: { p50: 4 }, worker: { mean: 2 }, outcome: 'content' },
    { kind: 'occupied-states', costScoreMs: 9, mainDelta: { p50: 3 }, worker: { mean: 1 }, outcome: 'content' },
    { kind: 'broken-kind', costScoreMs: null, mainDelta: { p50: null }, worker: { mean: null }, outcome: 'error' },
  ] };
  assert.deepEqual(heaviestKinds(costs, 2), ['scoreboard', 'occupied-states']);
  assert.throws(() => heaviestKinds(costs, 4), /requested 4 kinds.*only 3 measured kinds/);
  assert.throws(() => heaviestKinds({ widgets: [] }, 5), /requested 5 kinds.*only 0 measured kinds/);
  const noiseExcluded = { widgets: [
    { kind: 'scoreboard', costScoreMs: 12, mainDelta: { p50: 4, p95: 10 }, worker: { p95: 2, mean: 1 }, belowBaselineNoise: false },
    { kind: 'research', costScoreMs: 1.5, mainDelta: { p50: 1, p95: 1.5 }, worker: { p95: 0, mean: 0 }, belowBaselineNoise: false },
    { kind: 'occupied-states', costScoreMs: 0.5, mainDelta: { p50: 0, p95: -2.0 }, worker: { p95: 2.5, mean: 1 }, belowBaselineNoise: true },
  ] };
  assert.throws(() => heaviestKinds(noiseExcluded, 2), /below baseline noise.*occupied-states/);
  const acknowledged = selectHeaviestKinds(noiseExcluded, 2);
  assert.deepEqual(acknowledged.kinds, ['scoreboard', 'research']);
  assert.equal(acknowledged.excludedByNoise.length, 1);
  assert.match(acknowledged.warning, /occupied-states/);
  assert.deepEqual(heaviestKinds(noiseExcluded, 2, { allowBelowNoiseCutoff: true }), ['scoreboard', 'research']);
  const multiViewNoiseExcluded = { widgets: [
    { kind: 'scoreboard', costScoreMs: 12, mainDelta: { p50: 4, p95: 10 }, worker: { p95: 2, mean: 1 }, belowBaselineNoise: false },
    { kind: 'research', costScoreMs: 5, mainDelta: { p50: 2, p95: 5 }, worker: { p95: 0, mean: 0 }, belowBaselineNoise: false },
    { kind: 'naval-supremacy', view: 'shares', costScoreMs: 4, mainDelta: { p50: 1, p95: 4 }, worker: { p95: 0, mean: 0 }, belowBaselineNoise: false },
    { kind: 'naval-supremacy', view: 'timeline', costScoreMs: 3, mainDelta: { p50: -1, p95: -3 }, worker: { p95: 6, mean: 4 }, belowBaselineNoise: true },
    { kind: 'naval-supremacy', view: 'alt', costScoreMs: 2.5, mainDelta: { p50: -1, p95: -3 }, worker: { p95: 5.5, mean: 3 }, belowBaselineNoise: true },
  ] };
  assert.throws(() => heaviestKinds(multiViewNoiseExcluded, 2), /below baseline noise.*naval-supremacy\/timeline/);
  const multiViewNoiseSelection = selectHeaviestKinds(multiViewNoiseExcluded, 2);
  assert.equal(multiViewNoiseSelection.excludedByNoise.length, 1);
  assert.equal(multiViewNoiseSelection.excludedByNoise[0].kind, 'naval-supremacy');
  assert.equal(multiViewNoiseSelection.excludedByNoise[0].view, 'timeline');
  const unambiguousNegativeCutoff = { widgets: [
    { kind: 'scoreboard', costScoreMs: 12, mainDelta: { p50: 4, p95: 10 }, worker: { p95: 2, mean: 1 }, belowBaselineNoise: false },
    { kind: 'occupied-states', costScoreMs: 4.8, mainDelta: { p50: 0, p95: -0.2 }, worker: { p95: 5.0, mean: 3 }, belowBaselineNoise: true },
    { kind: 'wars', costScoreMs: -0.3, mainDelta: { p50: 0, p95: -0.5 }, worker: { p95: 0.2, mean: 0.1 }, belowBaselineNoise: true },
  ] };
  assert.deepEqual(heaviestKinds(unambiguousNegativeCutoff, 2), ['scoreboard', 'occupied-states'],
    'an excluded kind whose zero-clamped score stays below the cutoff is not flagged as a noise exclusion');
  const tied = { widgets: [
    { kind: 'alpha', costScoreMs: 10, mainDelta: { p50: 1 }, worker: { mean: 0 }, requestsPerSec: 0, commitsPerSec: 0 },
    { kind: 'beta', costScoreMs: 10, mainDelta: { p50: 5 }, worker: { mean: 1 }, requestsPerSec: 0, commitsPerSec: 0 },
  ] };
  assert.deepEqual(heaviestKinds(tied, 2), ['beta', 'alpha']);
  const withHeaviest = await allWidgetPages({ costs, heaviestCount: 2 });
  assert.ok(withHeaviest.pages[HEAVIEST]);
  assert.deepEqual(withHeaviest.pages[HEAVIEST].widgets.map(w => w.kind).sort(), ['occupied-states', 'scoreboard']);
});

test('the manifest enumerator yields every registered view exactly once and drives both allWidgetPages and costPages', async () => {
  const [{ widgetMetas }, familyManifest, manifest, families, storage, pages] = await Promise.all([
    import('../../ui/src/widgets/definitions.ts'),
    import('../../ui/src/widgets/familyManifest.ts'),
    import('../../ui/src/widgets/manifest.ts'),
    import('../../ui/src/families/manifest.ts'),
    import('../../ui/src/dashboard/storage.ts'),
    import('../../ui/src/dashboard/pages.ts'),
  ]);
  const expectedPairs = [
    ...familyManifest.FAMILY_VIEWS.map(entry => `${entry.kind}\0${families.normalizeView(entry.view)}`),
    ...manifest.NATIVE_WIDGETS.map(entry => `${entry.kind}\0${families.DEFAULT_VIEW}`),
  ].sort();
  assert.equal(new Set(expectedPairs).size, expectedPairs.length, 'manifest registers unique (kind, view) pairs');

  const allPlacements = await enumerateWidgetPlacements({ mode: 'all-views' });
  const actualPairs = allPlacements.map(entry => `${entry.kind}\0${entry.view}`);
  assert.equal(new Set(actualPairs).size, actualPairs.length, 'enumerator yields each registered view at most once');
  assert.deepEqual([...actualPairs].sort(), expectedPairs, 'enumerator yields every registered family view plus native widgets exactly once');
  for (const placement of allPlacements) {
    const resolved = manifest.resolveWidgetActiveView(placement.kind, placement.options, placement.meta);
    assert.equal(resolved, placement.view, `${placement.kind}/${placement.view} options resolve to its registered view`);
    if (placement.isDefaultView) assert.equal(placement.options, undefined, `${placement.kind} default view needs no stored option override`);
  }

  const defaultPlacements = await enumerateWidgetPlacements();
  assert.equal(defaultPlacements.length, widgetMetas.length);
  assert.deepEqual(defaultPlacements.map(entry => entry.kind), widgetMetas.map(meta => meta.kind));
  assert.ok(defaultPlacements.every(entry => entry.isDefaultView && entry.options === undefined));

  const everyViewFixture = await allWidgetPages({ allViews: true });
  const savedAll = new Map(everyViewFixture.entries);
  const readAll = { getItem: key => savedAll.get(key) ?? null, setItem: () => { throw new Error('read only'); } };
  for (const label of [FIXTURE, LEGIBLE]) {
    const page = everyViewFixture.pages[label];
    const { library, warning } = storage.loadLayouts(readAll, page.id);
    assert.equal(warning, null, `${label} every-view page restores cleanly`);
    const widgets = library.layouts[0].template.widgets;
    const restoredPairs = widgets.map(widget => `${widget.kind}\0${manifest.resolveWidgetActiveView(widget.kind, widget.options)}`).sort();
    assert.deepEqual(restoredPairs, expectedPairs, `${label} places every registered view once`);
  }

  const everyViewCosts = await costPages(null, { allViews: true });
  assert.equal(everyViewCosts.kinds.length, expectedPairs.length);
  const savedCost = new Map(everyViewCosts.entries);
  const readCost = { getItem: key => savedCost.get(key) ?? null, setItem: () => { throw new Error('read only'); } };
  const costPageList = pages.loadPages(readCost);
  assert.equal(costPageList.length, expectedPairs.length + 1, 'costPages every-view builds CostEmpty plus one page per registered view');
  const costPairs = [];
  for (const entry of everyViewCosts.kinds) {
    const { library, warning } = storage.loadLayouts(readCost, entry.id);
    assert.equal(warning, null, `${entry.label} restores cleanly`);
    const widget = library.layouts[0].template.widgets[0];
    costPairs.push(`${widget.kind}\0${manifest.resolveWidgetActiveView(widget.kind, widget.options)}`);
  }
  assert.deepEqual(costPairs.sort(), expectedPairs, 'costPages every-view covers every registered view once');

  const multiViewCosts = { widgets: [
    { kind: 'naval-supremacy', view: 'shares', costScoreMs: 4, mainDelta: { p50: 1 }, worker: { mean: 0 }, outcome: 'content' },
    { kind: 'naval-supremacy', view: 'timeline', costScoreMs: 18, mainDelta: { p50: 6 }, worker: { mean: 2 }, outcome: 'content' },
    { kind: 'scoreboard', costScoreMs: 10, mainDelta: { p50: 3 }, worker: { mean: 1 }, outcome: 'content' },
  ] };
  const heaviest = await allWidgetPages({ costs: multiViewCosts, heaviestCount: 2 });
  const savedHeaviest = new Map(heaviest.entries);
  const readHeaviest = { getItem: key => savedHeaviest.get(key) ?? null, setItem: () => { throw new Error('read only'); } };
  const heaviestWidgets = storage.loadLayouts(readHeaviest, heaviest.pages[HEAVIEST].id).library.layouts[0].template.widgets;
  const naval = heaviestWidgets.find(widget => widget.kind === 'naval-supremacy');
  const score = heaviestWidgets.find(widget => widget.kind === 'scoreboard');
  assert.equal(manifest.resolveWidgetActiveView(naval.kind, naval.options), 'timeline', 'Heaviest mounts the winning secondary view');
  assert.equal(score.options, undefined, 'Heaviest keeps default placement when cost row omits view');
});
