import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { classifyAgainstBase, formatReport, runCheckAtCommit, runDeadCodeCheck } from './check-dead-code.mjs';

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
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS];
    `,
    'ui/src/styles.css': "@import './theme.css'; body {}",
    'ui/src/theme.css': ':root {}',
    'ui/src/types.ts': 'export interface SharedType { product: string; head: string; }',
    'ui/src/inlineType.ts': 'export interface Inline { ok: boolean; }',
    'ui/src/lazy.ts': 'export default function Lazy() { return 1; } export const value = 1;',
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
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
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
    'ui/src/lazy.ts': 'export default function Lazy() { return 1; } export const value = 1; export const family = { kind: "kind-collision" };',
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
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 3, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
  }), async root => {
    const before = await runDeadCodeCheck(root);
    assert.equal(before.legacy.count, 3);
    assert.deepEqual(before.legacy.pairs, ['multi:lines', 'multi:mix', 'multi:race']);
    assert.deepEqual(before.legacy.errors, []);
  });

  await withRepo(baseFiles({
    'ui/src/widgets/definitions.ts': definitions,
    'ui/src/widgets/multi/raceProj.ts': 'export const proj = 1;',
    'ui/src/widgets/manifest/bars.ts': `
      export const BARS_VIEWS = [
        { kind: 'multi', view: 'race', family: 'bars', loadSpec: () => import('../multi/raceProj.ts') },
      ];
    `,
    'ui/src/widgets/familyManifest.ts': "import { BARS_VIEWS } from './manifest/bars.ts'; export const FAMILY_VIEWS = [...BARS_VIEWS];",
    'ui/src/widgets/legacyManifest.ts': `
      export const LEGACY_WIDGETS = [
        { kind: "multi", view: "lines", load: () => import("../lazy.ts") },
        { kind: "multi", view: "mix", load: () => import("../lazy.ts") },
      ];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 3, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
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
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a", load: () => import("../lazy.ts") }, { kind: "b", load: () => import("../lazy.ts") }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
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
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
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
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a", load: () => import("../lazy.ts") }, { kind: "b", load: () => import("../lazy.ts") }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
  }), runDeadCodeCheck);

  const unchanged = await withRepo(baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a", load: () => import("../lazy.ts") }, { kind: "b", load: () => import("../lazy.ts") }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
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
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "a", load: () => import("../lazy.ts") }, { kind: "b", load: () => import("../lazy.ts") }, { kind: "c", view: "lines", load: () => import("../lazy.ts") }];',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
  }), runDeadCodeCheck);

  const addedCmp = classifyAgainstBase(withAdded, base, 'abc123456789');
  assert.equal(addedCmp.newIssues, 2);
  assert.equal(addedCmp.inheritedIssues, 1);
  assert.deepEqual(addedCmp.addedLegacyPairs, ['c:lines']);
  const addedReport = formatReport(withAdded, addedCmp);
  assert.match(addedReport, /newOrphan\.ts \[new on this branch\]/);
  assert.match(addedReport, /added: c:lines\) \[new on this branch\]/);
});

test('D1: blocks ceiling raises, missing ratchet fields, unused-export swaps, and spending deletion headroom', async () => {
  await withRepo(baseFiles({
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1}',
  }), async root => {
    await assert.rejects(runDeadCodeCheck(root), /maxLegacyRendererViews/);
  });

  const base = await withRepo(baseFiles({
    'ui/src/mod.ts': 'export const live = 1; export const oldDeadA = 2; export const oldDeadB = 3;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { live } from './mod.ts';
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, live];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 2}',
  }), runDeadCodeCheck);
  assert.equal(base.totalIssues, 0);

  const raisedCeiling = await withRepo(baseFiles({
    'ui/src/mod.ts': 'export const live = 1; export const oldDeadA = 2; export const oldDeadB = 3; export const sneaky = 4;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { live } from './mod.ts';
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, live];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 3}',
  }), runDeadCodeCheck);
  assert.equal(raisedCeiling.totalIssues, 0);
  const raisedCmp = classifyAgainstBase(raisedCeiling, base, 'abc123456789');
  assert.ok(raisedCmp.newIssues >= 2);
  assert.ok(raisedCmp.ratchetRaises.some(r => r.includes('maxUnusedExports raised from 2 to 3')));
  assert.deepEqual(raisedCmp.addedUnusedExports.map(e => e.key), ['ui/src/mod.ts:sneaky']);

  const swappedAndSpentHeadroom = await withRepo(baseFiles({
    'ui/src/mod.ts': 'export const live = 1; export const swappedNew = 4;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { live } from './mod.ts';
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, live];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 1}',
  }), runDeadCodeCheck);
  assert.equal(swappedAndSpentHeadroom.totalIssues, 0);
  const swapCmp = classifyAgainstBase(swappedAndSpentHeadroom, base, 'abc123456789');
  assert.equal(swapCmp.newIssues, 1);
  assert.deepEqual(swapCmp.addedUnusedExports.map(e => e.key), ['ui/src/mod.ts:swappedNew']);
});

test('D2: resolves mapped shorthand kind entries and fails on unresolvable runtime FAMILY_VIEWS pairs', async () => {
  await withRepo(baseFiles({
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { FAMILY_VIEWS } from './widgets/familyManifest.ts';
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, FAMILY_VIEWS];
    `,
    'ui/src/charts/OldChart.tsx': 'export const OldChart = () => null;',
    'ui/src/widgets/power/powerProjection.ts': "import { OldChart } from '../../charts/OldChart.tsx'; export const projA = () => OldChart(); export const projB = () => OldChart();",
    'ui/src/widgets/manifest/table.ts': `
      function tableSpec(kind: string, view: string, load: () => Promise<any>, pick: (m: any) => any) {
        return () => load().then(pick);
      }
      export const TABLE_VIEWS = [
        ...(['political-power-spending', 'command-power-spending'] as const).map((kind) => ({
          kind,
          view: 'default',
          family: 'table',
          loadSpec: tableSpec(kind, 'default', () => import('../power/powerProjection.ts'), mod => (kind === 'political-power-spending' ? mod.projA : mod.projB)),
        })),
      ];
    `,
    'ui/src/widgets/familyManifest.ts': "import { TABLE_VIEWS } from './manifest/table.ts'; export const FAMILY_VIEWS = [...TABLE_VIEWS];",
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(
      result.convertedLegacy.violations.map(v => v.pair),
      ['command-power-spending:default', 'political-power-spending:default'],
    );
    assert.deepEqual(result.convertedLegacy.unresolvedFamilyViews, []);
  });

  await withRepo(baseFiles({
    'ui/src/widgets/familyManifest.ts': `
      const opaqueLoader = (p: string) => () => import(p);
      export const FAMILY_VIEWS = [{ kind: 'mystery', view: 'default', family: 'table', loadSpec: opaqueLoader('../lazy.ts') }];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.convertedLegacy.unresolvedFamilyViews.map(v => v.pair), ['mystery:default']);
    assert.ok(result.convertedLegacy.errors.some(e => e.includes('unresolved FAMILY_VIEWS entry mystery:default')));
  });
});

test('D3: detects old charts through family helpers, ignores pure chart utils, and catches widget-only legacy React renderers', async () => {
  await withRepo(baseFiles({
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { FAMILY_VIEWS } from './widgets/familyManifest.ts';
      import { FAMILY_VIEW_UI } from './widgets/manifest.ts';
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, FAMILY_VIEWS, FAMILY_VIEW_UI];
    `,
    'ui/src/charts/pureUtil.ts': 'export const pureMath = () => 1;',
    'ui/src/charts/OldChart.tsx': 'export const OldChart = () => null;',
    'ui/src/families/table/helper.ts': "import { OldChart } from '../../charts/OldChart.tsx'; export const helper = () => OldChart();",
    'ui/src/widgets/old-react/OldReactWidget.tsx': 'export const OldReactWidget = () => null;',
    'ui/src/widgets/old-react/index.ts': "import { OldReactWidget } from './OldReactWidget.tsx'; export default OldReactWidget;",
    'ui/src/widgets/clean-util/cleanProjection.ts': "import { pureMath } from '../../charts/pureUtil.ts'; export const proj = () => pureMath();",
    'ui/src/widgets/via-family/viaFamilyProjection.ts': "import { helper } from '../../families/table/helper.ts'; export const proj = () => helper();",
    'ui/src/widgets/reaches-widget/reachesProjection.ts': "import { OldReactWidget } from '../old-react/OldReactWidget.tsx'; export const proj = () => OldReactWidget();",
    'ui/src/widgets/legacyManifest.ts': 'export const LEGACY_WIDGETS = [{ kind: "old-react", load: () => import("./old-react/index.ts") }];',
    'ui/src/widgets/manifest/table.ts': `
      export const TABLE_VIEWS = [
        { kind: 'clean-util', view: 'default', family: 'table', loadSpec: () => import('../clean-util/cleanProjection.ts').then(m => m.proj) },
        { kind: 'via-family', view: 'default', family: 'table', loadSpec: () => import('../via-family/viaFamilyProjection.ts').then(m => m.proj) },
        { kind: 'reaches-widget', view: 'default', family: 'table', loadSpec: () => import('../reaches-widget/reachesProjection.ts').then(m => m.proj) },
      ];
    `,
    'ui/src/widgets/familyManifest.ts': "import { TABLE_VIEWS } from './manifest/table.ts'; export const FAMILY_VIEWS = [...TABLE_VIEWS];",
    'ui/src/widgets/manifest.ts': 'export const FAMILY_VIEW_UI = {};',
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 10}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(
      result.convertedLegacy.violations.map(v => v.pair),
      ['reaches-widget:default', 'via-family:default'],
    );
    assert.deepEqual(
      result.convertedLegacy.violations[0].chain,
      ['ui/src/widgets/reaches-widget/reachesProjection.ts', 'ui/src/widgets/old-react/OldReactWidget.tsx'],
    );
    assert.deepEqual(
      result.convertedLegacy.violations[1].chain,
      ['ui/src/widgets/via-family/viaFamilyProjection.ts', 'ui/src/families/table/helper.ts', 'ui/src/charts/OldChart.tsx'],
    );
  });
});

test('D4: expands explicit view default, evaluates spread legacy entries, and uses view option over viewSwitch', async () => {
  await withRepo(baseFiles({
    'ui/src/widgets/definitions.ts': `
      export const widgetMetas = [
        {
          kind: 'dockyard-spending',
          options: [{ key: 'view', kind: 'choice', choices: [{ value: 'circle' }, { value: 'months' }] }],
        },
        {
          kind: 'equipment-produced',
          viewSwitch: { option: 'count', choices: [{ value: 'units' }, { value: 'ic' }] },
          options: [
            { key: 'count', kind: 'choice', choices: [{ value: 'units' }, { value: 'ic' }] },
            { key: 'view', kind: 'choice', choices: [{ value: 'race' }, { value: 'lines' }] },
          ],
        },
      ];
    `,
    'ui/src/widgets/legacyManifest.ts': `
      const extra = [{ kind: 'spread-widget', load: () => import('../lazy.ts') }];
      export const LEGACY_WIDGETS = [
        { kind: 'dockyard-spending', view: 'default', load: () => import('../lazy.ts') },
        { kind: 'equipment-produced', load: () => import('../lazy.ts') },
        ...extra,
      ];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 5, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.legacy.pairs, [
      'dockyard-spending:circle',
      'dockyard-spending:months',
      'equipment-produced:lines',
      'equipment-produced:race',
      'spread-widget:default',
    ]);
    assert.equal(result.legacy.count, 5);
  });
});

test('D5: uses vite shipping entries, separates type/runtime/test reachability, and ignores unused re-exports and stale legacy loaders', async () => {
  await withRepo(baseFiles({
    'ui/vite.config.ts': `
      export default {
        build: { rolldownOptions: { input: { main: new URL('index.html', import.meta.url) } } },
      };
    `,
    'ui/scratch.html': '<!doctype html><script type="module" src="/src/scratchOnly.ts"></script>',
    'ui/src/scratchOnly.ts': 'export const scratch = 1;',
    'ui/src/typeHolder.ts': "import { runtimeLeak } from './runtimeLeak.ts'; export interface UsefulType { ok: boolean; } export const unusedRuntime = () => runtimeLeak();",
    'ui/src/runtimeLeak.ts': 'export const runtimeLeak = () => 1;',
    'ui/src/barrel.ts': "export { liveFromBarrel } from './liveSub.ts'; export { deadFromBarrel } from './deadSub.ts';",
    'ui/src/liveSub.ts': 'export const liveFromBarrel = () => 1;',
    'ui/src/deadSub.ts': 'export const deadFromBarrel = () => 2;',
    'ui/src/staleTarget.ts': 'export default function Stale() { return 0; }',
    'ui/src/widgets/converted/proj.ts': 'export const proj = 1;',
    'ui/src/widgets/manifest/table.ts': "export const TABLE_VIEWS = [{ kind: 'converted', view: 'default', family: 'table', loadSpec: () => import('../converted/proj.ts').then(m => m.proj) }];",
    'ui/src/widgets/familyManifest.ts': "import { TABLE_VIEWS } from './manifest/table.ts'; export const FAMILY_VIEWS = [...TABLE_VIEWS];",
    'ui/src/widgets/legacyManifest.ts': `
      export const LEGACY_WIDGETS = [
        { kind: 'a', load: () => import('../lazy.ts') },
        { kind: 'converted', load: () => import('../staleTarget.ts') },
      ];
    `,
    'ui/src/sneaky.fixture.tsx': "import { behindFixture } from './behindFixture.ts'; export const fx = () => behindFixture();",
    'ui/src/behindFixture.ts': 'export const behindFixture = () => 9;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import type { UsefulType } from './typeHolder.ts';
      import { liveFromBarrel } from './barrel.ts';
      import { fx } from './sneaky.fixture.tsx';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { FAMILY_VIEWS } from './widgets/familyManifest.ts';
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline & UsefulType;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, FAMILY_VIEWS, liveFromBarrel(), fx()];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 10}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.entryPoints, ['ui/index.html']);
    assert.deepEqual(result.ui.deadProduction, [
      'ui/src/behindFixture.ts',
      'ui/src/deadSub.ts',
      'ui/src/runtimeLeak.ts',
      'ui/src/scratchOnly.ts',
      'ui/src/staleTarget.ts',
    ]);
    assert.deepEqual(result.ui.productionTestImports, [
      { from: 'ui/src/main.tsx', to: 'ui/src/sneaky.fixture.tsx', role: 'fixture' },
    ]);
    assert.deepEqual(result.legacy.staleEntries, ['converted:default']);
  });
});

test('D6: resolves template/commented dynamic imports, ignores unused named imports and shadowed namespaces, and tracks loader callers', async () => {
  await withRepo(baseFiles({
    'ui/src/commented.ts': 'export const ok = 1;',
    'ui/src/templated.ts': 'export const ok = 2;',
    'ui/src/chart.ts': 'export const draw = () => 1; export const unusedChartExport = () => 2;',
    'ui/src/nsTarget.ts': 'export const usedNs = 1; export const unusedNs = 2;',
    'ui/src/unusedTarget.ts': 'export const neverUsed = 1;',
    'ui/src/lazyComp.ts': 'export default function LazyComp() { return 1; } export const deadOnLazy = 2;',
    'ui/src/consumer.ts': `
      import { neverUsed } from './unusedTarget.ts';
      import * as ns from './nsTarget.ts';
      const loadChart = () => import('./chart.ts');
      const loadCommented = async () => (await import /* comment */ ('./commented.ts')).ok;
      const loadTemplated = async () => (await import(\`./templated.ts\`)).ok;
      export const shadow = (ns: number) => ns + 1;
      export const useNs = () => { const { usedNs } = ns; return usedNs; };
      export const runChart = async () => {
        const mod = await loadChart();
        return mod.draw() + (await loadCommented()) + (await loadTemplated());
      };
    `,
    'ui/src/main.tsx': `
      import './styles.css';
      import { lazy } from 'react';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
      import { runChart, shadow, useNs } from './consumer.ts';
      export const Lazy = lazy(() => import('./lazyComp.ts'));
      export const loadLazy = async () => (await import('./lazy.ts')).value;
      export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      export type Ref = import('./inlineType.ts').Inline;
      export const used: SharedType = { product: 'live-product', head: 'live-head' };
      export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, Lazy, runChart(), shadow(1), useNs()];
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.deadProduction, ['ui/src/unusedTarget.ts']);
    assert.deepEqual(
      result.exports.unusedExports.map(e => e.key),
      [
        'ui/src/chart.ts:unusedChartExport',
        'ui/src/lazyComp.ts:deadOnLazy',
        'ui/src/nsTarget.ts:unusedNs',
      ],
    );
  });
});

test('D7 and drive-letter snapshot: runCheckAtCommit unpacks nested tool callers from a temp path with drive letter on Windows', async () => {
  await withRepo(baseFiles({
    'ui/src/generated/dashboard.ts': `
export const REQUEST_PATHS = {
  "live-product": {},
  "head:live-head": {},
  "tool-product": {},
  "nested-tool-product": {},
  "crate-product": {},
};
`,
    'tools/e2e/nestedCaller.mjs': 'export const prod = "nested-tool-product";',
  }), async root => {
    if (process.platform === 'win32') {
      assert.match(root, /^[A-Za-z]:\\/);
    }
    const git = args => execFileSync('git', args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] }).toString('utf8').trim();
    git(['init', '-q', '-b', 'main']);
    git(['add', '-A']);
    git(['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-m', 'base']);
    const baseSha = git(['rev-parse', 'HEAD']);

    const baseResult = await runCheckAtCommit(root, baseSha);
    assert.deepEqual(baseResult.rust.deadProducts, []);

    rmSync(join(root, 'tools/e2e/nestedCaller.mjs'));
    const headResult = await runDeadCodeCheck(root);
    assert.deepEqual(headResult.rust.deadProducts.map(p => p.product), ['nested-tool-product']);

    const cmp = classifyAgainstBase(headResult, baseResult, baseSha);
    assert.equal(cmp.newIssues, 1);
    assert.equal(cmp.inheritedIssues, 0);
    assert.equal(cmp.deadProducts[0].inherited, false);
  });
});

test('missing ratchet file at base commit is read as no ratchet instead of throwing', async () => {
  const files = baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
  });
  delete files['tools/legacy-widgets-ratchet.json'];
  delete files['ui/src/widgets/legacyManifest.ts'];
  files['ui/src/main.tsx'] = `
    import './styles.css';
    import type { SharedType } from './types.ts';
    import { REQUEST_PATHS } from './generated/dashboard.ts';
    export const loadLazy = async () => (await import('./lazy.ts')).value;
    export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
    export type Ref = import('./inlineType.ts').Inline;
    export const used: SharedType = { product: 'live-product', head: 'live-head' };
    export const refs = [REQUEST_PATHS];
  `;
  await withRepo(files, async root => {
    const git = args => execFileSync('git', args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] }).toString('utf8').trim();
    git(['init', '-q', '-b', 'main']);
    git(['add', '-A']);
    git(['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-m', 'pre-ratchet base']);
    const baseSha = git(['rev-parse', 'HEAD']);

    const baseResult = await runCheckAtCommit(root, baseSha);
    assert.equal(baseResult.ratchetMissing, true);
    assert.deepEqual(baseResult.ui.deadProduction, ['ui/src/inheritedOrphan.ts']);

    writeFileSync(
      join(root, 'tools/legacy-widgets-ratchet.json'),
      '{"maxLegacyViews": 1, "maxLegacyRendererViews": 0, "maxUnusedExports": 1}',
      'utf8',
    );
    mkdirSync(join(root, 'ui/src/widgets'), { recursive: true });
    writeFileSync(
      join(root, 'ui/src/widgets/legacyManifest.ts'),
      'export const LEGACY_WIDGETS = [{ kind: "a", load: () => import("../lazy.ts") }];',
      'utf8',
    );
    writeFileSync(
      join(root, 'ui/src/main.tsx'),
      `
        import './styles.css';
        import type { SharedType } from './types.ts';
        import { REQUEST_PATHS } from './generated/dashboard.ts';
        import { LEGACY_WIDGETS } from './widgets/legacyManifest.ts';
        import { live } from './mod.ts';
        export const loadLazy = async () => (await import('./lazy.ts')).value;
        export const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
        export type Ref = import('./inlineType.ts').Inline;
        export const used: SharedType = { product: 'live-product', head: 'live-head' };
        export const refs = [REQUEST_PATHS, LEGACY_WIDGETS, live];
      `,
      'utf8',
    );
    writeFileSync(
      join(root, 'ui/src/mod.ts'),
      'export const live = 1; export const unusedAtHead = 2;',
      'utf8',
    );

    const headResult = await runDeadCodeCheck(root);
    assert.equal(headResult.ratchetMissing, false);

    const cmp = classifyAgainstBase(headResult, baseResult, baseSha);
    assert.deepEqual(cmp.ratchetRaises, []);
    assert.deepEqual(cmp.addedLegacyPairs, ['a:default']);
    assert.deepEqual(cmp.addedUnusedExports.map(e => e.key), ['ui/src/mod.ts:unusedAtHead']);
    assert.equal(cmp.deadProduction[0].inherited, true);
    assert.equal(cmp.newIssues, 2);
  });
});
