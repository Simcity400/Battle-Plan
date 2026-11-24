import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { classifyAgainstBase, formatReport, runDeadCodeCheck } from './check-dead-code.mjs';

async function withRepo(files, fn) {
  const root = mkdtempSync(join(tmpdir(), 'bp-dead-code-'));
  try {
    for (const [rel, content] of Object.entries(files)) {
      const full = join(root, rel);
      mkdirSync(dirname(full), { recursive: true });
      writeFileSync(full, content, 'utf8');
    }
    return await fn(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

function baseFiles(overrides = {}) {
  return {
    'ui/index.html': '<!doctype html><script type="module" src="/src/main.tsx"></script>',
    'ui/src/env.d.ts': 'declare const __FLAG__: boolean;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      export const loadLazy = () => import('./lazy.ts');
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS];
    `,
    'ui/src/styles.css': "@import './theme.css'; body {}",
    'ui/src/theme.css': ':root {}',
    'ui/src/types.ts': 'export interface SharedType { product: string; head: string; }',
    'ui/src/inlineType.ts': 'export interface Inline { ok: boolean; }',
    'ui/src/lazy.ts': 'export const value = 1;',
    'ui/src/lane.worker.ts': 'self.postMessage("ready");',
    'ui/src/generated/dashboard.ts': `
export const REQUEST_PATHS = {
  "live-product": {},
  "head:live-head": {},
  "tool-product": {},
  "crate-product": {},
};
`,
    'crates/dashboard/src/products/mod.rs': 'const OWN: &[Registered] = &[dashboard_world::registrations::LIVE];',
    'crates/dashboard-world/src/registrations.rs': 'dashboard_base::register_products! { LIVE => crate::Live, }',
    'tools/helper.mjs': 'export const used = "tool-product";',
    'crates/desktop/src/lib.rs': 'pub const USED: &str = "crate-product";',
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a", load: () => import("../lazy.ts") }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1}',
    ...overrides,
  };
}

test('passes on a clean graph with static, dynamic, type, worker, CSS, tool, and crate callers', async () => {
  await withRepo(baseFiles({
    'ui/src/live.test.ts': `
      import { value } from './lazy.ts';
      import { helper } from './test-support/helper.ts';
      export const run = () => [value, helper, '/ui/src/live.dom.fixture.tsx'];
    `,
    'ui/src/live.dom.fixture.tsx': 'export const fixture = 1;',
    'ui/src/test-support/helper.ts': 'export const helper = 2;',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.equal(result.totalIssues, 0);
    assert.deepEqual(result.ui.deadProduction, []);
    assert.deepEqual(result.ui.deadTests, []);
    assert.deepEqual(result.ui.deadFixturesAndSupport, []);
    assert.deepEqual(result.rust.deadProducts, []);
    assert.deepEqual(result.rust.unwiredRegistrations, []);
    assert.deepEqual(result.legacy.errors, []);
  });
});

test('flags dead UI modules, modules only imported by tests, those tests, and orphaned fixtures', async () => {
  await withRepo(baseFiles({
    'ui/src/orphan.ts': 'export const dead = 1;',
    'ui/src/testOnly.ts': 'export const onlyTested = 2;',
    'ui/src/testOnly.test.ts': "import { onlyTested } from './testOnly.ts'; export const x = onlyTested;",
    'ui/src/unused.dom.fixture.tsx': 'export const unused = 3;',
    'ui/src/test-support/unused.ts': 'export const unusedSupport = 4;',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.deadProduction, ['ui/src/orphan.ts', 'ui/src/testOnly.ts']);
    assert.deepEqual(result.ui.deadTests, [{ test: 'ui/src/testOnly.test.ts', targets: ['ui/src/testOnly.ts'] }]);
    assert.deepEqual(result.ui.deadFixturesAndSupport, [
      { file: 'ui/src/test-support/unused.ts', role: 'test-support' },
      { file: 'ui/src/unused.dom.fixture.tsx', role: 'fixture' },
    ]);
  });
});

test('flags dead Rust products, products only called by dead/test UI, series-kind name collisions, and unwired registrations', async () => {
  await withRepo(baseFiles({
    'ui/src/generated/dashboard.ts': `
export const REQUEST_PATHS = {
  "live-product": {},
  "head:live-head": {},
  "tool-product": {},
  "crate-product": {},
  "dead-product": {},
  "only-in-dead-ui": {},
  "kind-collision": {},
};
`,
    'ui/src/lazy.ts': 'export const value = 1; export const family = { kind: "kind-collision" };',
    'ui/src/orphan.ts': 'export const p = "only-in-dead-ui";',
    'crates/dashboard-world/src/registrations.rs': 'dashboard_base::register_products! { LIVE => crate::Live, UNWIRED => crate::Unwired, }',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.rust.deadProducts, [
      { product: 'dead-product', onlyIn: [] },
      { product: 'kind-collision', onlyIn: [] },
      { product: 'only-in-dead-ui', onlyIn: ['ui/src/orphan.ts'] },
    ]);
    assert.deepEqual(result.rust.unwiredRegistrations, [
      'crates/dashboard-world/src/registrations.rs: UNWIRED',
    ]);
  });
});

test('counts legacy (kind, view) pairs from definitions so splitting one converted view lowers the count', async () => {
  const definitions = `
    export const widgetMetas = [
      {
        kind: 'multi',
        viewSwitch: { option: 'view', choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines' }, { value: 'mix', label: 'Mix' }] },
      },
    ];
  `;
  await withRepo(baseFiles({
    'ui/src/widgets/definitions.ts': definitions,
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "multi", load: () => import("../lazy.ts") }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 3}',
  }), async root => {
    const before = await runDeadCodeCheck(root);
    assert.equal(before.legacy.count, 3);
    assert.deepEqual(before.legacy.pairs, ['multi:lines', 'multi:mix', 'multi:race']);
    assert.deepEqual(before.legacy.errors, []);
  });

  await withRepo(baseFiles({
    'ui/src/widgets/definitions.ts': definitions,
    'ui/src/widgets/familyManifest.ts': 'export const FAMILY_VIEWS = [{ kind: "multi", view: "race" }];',
    'ui/src/widgets/legacyManifest.ts': `
      export const LEGACY_WIDGETS = [
        { kind: "multi", view: "lines", load: () => import("../lazy.ts") },
        { kind: "multi", view: "mix", load: () => import("../lazy.ts") },
      ];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 3}',
  }), async root => {
    const after = await runDeadCodeCheck(root);
    assert.equal(after.legacy.count, 2);
    assert.deepEqual(after.legacy.pairs, ['multi:lines', 'multi:mix']);
    assert.deepEqual(after.legacy.errors, []);
    assert.equal(after.legacy.notices.length, 1);
    assert.match(after.legacy.notices[0], /lower maxLegacyViews .* to 2/);
  });
});

test('enforces legacy view ratchet and requires deleting legacy infrastructure at zero', async () => {
  await withRepo(baseFiles({
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a" }, { kind: "b" }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.equal(result.legacy.errors.length, 1);
    assert.match(result.legacy.errors[0], /has 2 \(kind, view\) pair\(s\), above ratchet 1/);
  });

  await withRepo(baseFiles({
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [];',
    'ui/src/families/manifest.ts': 'export interface LegacyWidgetEntry { kind: string }',
    'ui/src/widgets/manifest.ts': 'export const legacyEntriesForKind = () => [];',
    'ui/src/widgets/rendererLoaders.ts': 'export const x = LEGACY_WIDGETS;',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.ok(result.legacy.errors.some(err => err.includes('legacyManifest.ts is empty')));
    assert.ok(result.legacy.errors.some(err => err.includes('LegacyWidgetEntry still exists')));
    assert.ok(result.legacy.errors.some(err => err.includes('legacyEntriesForKind still exists')));
    assert.ok(result.legacy.errors.some(err => err.includes('LEGACY_WIDGETS still exists')));
    assert.ok(result.legacy.errors.some(err => err.includes('from 1 to 0')));
  });
});

test('classifies inherited vs branch-added failures and names added legacy pairs', async () => {
  const base = await withRepo(baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a" }, { kind: "b" }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1}',
  }), runDeadCodeCheck);

  const unchanged = await withRepo(baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a" }, { kind: "b" }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1}',
  }), runDeadCodeCheck);

  const inheritedOnly = classifyAgainstBase(unchanged, base, 'abc123456789');
  assert.equal(inheritedOnly.newIssues, 0);
  assert.equal(inheritedOnly.inheritedIssues, 2);
  const inheritedReport = formatReport(unchanged, inheritedOnly);
  assert.match(inheritedReport, /inheritedOrphan\.ts \[inherited from abc123456\]/);
  assert.match(inheritedReport, /above ratchet 1 \[inherited from abc123456\]/);

  const withAdded = await withRepo(baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
    'ui/src/newOrphan.ts': 'export const added = 2;',
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a" }, { kind: "b" }, { kind: "c", view: "lines" }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1}',
  }), runDeadCodeCheck);

  const addedCmp = classifyAgainstBase(withAdded, base, 'abc123456789');
  assert.equal(addedCmp.newIssues, 2);
  assert.equal(addedCmp.inheritedIssues, 1);
  assert.deepEqual(addedCmp.addedLegacyPairs, ['c:lines']);
  const addedReport = formatReport(withAdded, addedCmp);
  assert.match(addedReport, /newOrphan\.ts \[new on this branch\]/);
  assert.match(addedReport, /added: c:lines\) \[new on this branch\]/);
});

test('flags converted views that reach old charts or legacy-only dashboard hooks by rule, while allowing family-shared kit', async () => {
  await withRepo(baseFiles({
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { FAMILY_VIEWS } from './widgets/familyManifest.ts';
      import { FAMILY_VIEW_UI } from './widgets/manifest.ts';
      export const loadLazy = () => import('./lazy.ts');
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, FAMILY_VIEWS, FAMILY_VIEW_UI];
    `,
    'ui/src/charts/sharedKit.ts': 'export const kitHelper = () => 1;',
    'ui/src/charts/oldChart.ts': 'export const oldRender = () => 2;',
    'ui/src/dashboard/legacyHook.tsx': 'export const useOld = () => null;',
    'ui/src/dashboard/sharedData.ts': 'export const parseRow = () => 3;',
    'ui/src/families/table/painter.ts': "import { kitHelper } from '../../charts/sharedKit.ts'; export const paint = () => kitHelper();",
    'ui/src/widgets/old-widget/index.ts': "import { oldRender } from '../../charts/oldChart.ts'; import { useOld } from '../../dashboard/legacyHook.tsx'; import { parseRow } from '../../dashboard/sharedData.ts'; export default () => [oldRender(), useOld(), parseRow()];",
    'ui/src/widgets/clean-widget/cleanProjection.ts': "import { kitHelper } from '../../charts/sharedKit.ts'; import { parseRow } from '../../dashboard/sharedData.ts'; export const proj = () => [kitHelper(), parseRow()];",
    'ui/src/widgets/dirty-chart/dirtyProjection.ts': "import { oldRender } from '../../charts/oldChart.ts'; export const proj = () => oldRender();",
    'ui/src/widgets/dirty-dash/cleanProj.ts': 'export const proj = () => 1;',
    'ui/src/widgets/dirty-dash/index.ts': "import { useOld } from '../../dashboard/legacyHook.tsx'; export default () => useOld();",
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "old-widget", load: () => import("./old-widget/index.ts") }];',
    'ui/src/widgets/manifest/table.ts': `
      import { paint } from '../../families/table/painter.ts';
      export const TABLE_VIEWS = [
        { kind: 'clean-widget', view: 'default', family: 'table', loadSpec: () => import('../clean-widget/cleanProjection.ts').then(m => ({ projection: m.proj, paint })) },
        { kind: 'dirty-chart', view: 'default', family: 'table', loadSpec: () => import('../dirty-chart/dirtyProjection.ts').then(m => ({ projection: m.proj })) },
        { kind: 'dirty-dash', view: 'default', family: 'table', loadSpec: () => import('../dirty-dash/cleanProj.ts').then(m => ({ projection: m.proj })) },
      ];
    `,
    'ui/src/widgets/familyManifest.ts': "import { TABLE_VIEWS } from './manifest/table.ts'; export const FAMILY_VIEWS = [...TABLE_VIEWS];",
    'ui/src/widgets/manifest.ts': "export const FAMILY_VIEW_UI = { 'dirty-dash': { load: () => import('./dirty-dash/index.ts') } };",
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 1}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(
      result.convertedLegacy.violations.map(v => v.pair),
      ['dirty-chart:default', 'dirty-dash:default'],
    );
    assert.deepEqual(
      result.convertedLegacy.violations[0].chain,
      ['ui/src/widgets/dirty-chart/dirtyProjection.ts', 'ui/src/charts/oldChart.ts'],
    );
    assert.deepEqual(
      result.convertedLegacy.violations[1].chain,
      ['ui/src/widgets/dirty-dash/index.ts', 'ui/src/dashboard/legacyHook.tsx'],
    );
    assert.equal(result.convertedLegacy.errors.length, 1);
    assert.match(result.convertedLegacy.errors[0], /2 converted view\(s\) reach old-renderer modules, above ratchet 1/);
  });
});

test('flags unused exports and dead re-exports while keeping locally used helpers, dynamic imports, and awaited imports', async () => {
  await withRepo(baseFiles({
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { usedFn } from './mod.ts';
      export const loadLazy = () => import('./lazy.ts');
      export const loadAwait = async () => (await import('./awaited.ts')).awaitedFn();
      export const loadDestruct = async () => { const { destructured } = await import('./awaited.ts'); return destructured; };
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, usedFn()];
    `,
    'ui/src/lazy.ts': 'export default function Lazy() { return 1; } export const deadOnLazy = 2;',
    'ui/src/awaited.ts': 'export const awaitedFn = () => 1; export const destructured = 2; export const deadAwaited = 3;',
    'ui/src/mod.ts': `
      export const localHelper = () => 1;
      export const usedFn = () => localHelper();
      export const deadDirect = () => 2;
      export const deadTransitive = () => deadHelper();
      export const deadHelper = () => 3;
      export { unusedReexport } from './retarget.ts';
    `,
    'ui/src/retarget.ts': 'export const unusedReexport = 4;',
    'ui/src/mod.test.ts': "import { deadDirect, localHelper } from './mod.ts'; export const run = () => [deadDirect(), localHelper()];",
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(
      result.exports.unusedExports.map(item => item.key),
      [
        'ui/src/awaited.ts:deadAwaited',
        'ui/src/lazy.ts:deadOnLazy',
        'ui/src/mod.ts:deadDirect',
        'ui/src/mod.ts:deadHelper',
        'ui/src/mod.ts:deadTransitive',
        'ui/src/mod.ts:unusedReexport',
        'ui/src/retarget.ts:unusedReexport',
      ],
    );
    assert.equal(result.exports.errors.length, 1);
    assert.match(result.exports.errors[0], /7 unused export\(s\) in ui\/src, above ratchet 0/);
  });
});
