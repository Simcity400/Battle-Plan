import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { classifyAgainstBase, compareWithBase, formatReport, parseAgainstArg, resolveBaseCommit, runCheckAtCommit, runDeadCodeCheck } from './check-dead-code.mjs';

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
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS);
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
    'tools/helper.mjs': 'export const used = "tool-product";',
    'crates/desktop/src/lib.rs': 'pub const USED: &str = "crate-product";',
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
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

test('flags dead Rust products, products only called by dead/test UI, and series-kind name collisions', async () => {
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
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.rust.deadProducts, [
      { product: 'dead-product', onlyIn: [] },
      { product: 'kind-collision', onlyIn: [] },
      { product: 'only-in-dead-ui', onlyIn: ['ui/src/orphan.ts'] },
    ]);
  });
});

test('classifies inherited vs branch-added failures', async () => {
  const base = await withRepo(baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
  }), runDeadCodeCheck);

  const unchanged = await withRepo(baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
  }), runDeadCodeCheck);

  const inheritedOnly = classifyAgainstBase(unchanged, base, 'abc123456789');
  assert.equal(inheritedOnly.newIssues, 0);
  assert.equal(inheritedOnly.inheritedIssues, 1);
  const inheritedReport = formatReport(unchanged, inheritedOnly);
  assert.match(inheritedReport, /inheritedOrphan\.ts \[inherited from abc123456\]/);

  const withAdded = await withRepo(baseFiles({
    'ui/src/inheritedOrphan.ts': 'export const old = 1;',
    'ui/src/newOrphan.ts': 'export const added = 2;',
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
  }), runDeadCodeCheck);

  const addedCmp = classifyAgainstBase(withAdded, base, 'abc123456789');
  assert.equal(addedCmp.newIssues, 1);
  assert.equal(addedCmp.inheritedIssues, 1);
  const addedReport = formatReport(withAdded, addedCmp);
  assert.match(addedReport, /newOrphan\.ts \[new on this branch\]/);
});

test('D1: blocks ceiling raises, missing ratchet fields, unused-export swaps, and spending deletion headroom', async () => {
  await withRepo(baseFiles({
    'tools/legacy-widgets-ratchet.json': '{}',
  }), async root => {
    await assert.rejects(runDeadCodeCheck(root), /maxUnusedExports/);
  });

  const base = await withRepo(baseFiles({
    'ui/src/mod.ts': 'export const live = 1; export const oldDeadA = 2; export const oldDeadB = 3;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { live } from './mod.ts';
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS, live);
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 2}',
  }), runDeadCodeCheck);
  assert.equal(base.totalIssues, 0);

  const raisedCeiling = await withRepo(baseFiles({
    'ui/src/mod.ts': 'export const live = 1; export const oldDeadA = 2; export const oldDeadB = 3; export const sneaky = 4;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { live } from './mod.ts';
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS, live);
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 3}',
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
      import { live } from './mod.ts';
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS, live);
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 1}',
  }), runDeadCodeCheck);
  assert.equal(swappedAndSpentHeadroom.totalIssues, 0);
  const swapCmp = classifyAgainstBase(swappedAndSpentHeadroom, base, 'abc123456789');
  assert.equal(swapCmp.newIssues, 1);
  assert.deepEqual(swapCmp.addedUnusedExports.map(e => e.key), ['ui/src/mod.ts:swappedNew']);
});

test('D5: uses vite shipping entries, separates type/runtime/test reachability, and ignores unused re-exports', async () => {
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
    'ui/src/widgets/converted/proj.ts': 'export const proj = 1;',
    'ui/src/widgets/manifest/table.ts': "export const TABLE_VIEWS = [{ kind: 'converted', view: 'default', family: 'table', loadSpec: () => import('../converted/proj.ts').then(m => m.proj) }];",
    'ui/src/widgets/familyManifest.ts': "import { TABLE_VIEWS } from './manifest/table.ts'; export const FAMILY_VIEWS = [...TABLE_VIEWS];",
    'ui/src/sneaky.fixture.tsx': "import { behindFixture } from './behindFixture.ts'; export const fx = () => behindFixture();",
    'ui/src/behindFixture.ts': 'export const behindFixture = () => 9;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import type { UsefulType } from './typeHolder.ts';
      import { liveFromBarrel } from './barrel.ts';
      import { fx } from './sneaky.fixture.tsx';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { FAMILY_VIEWS } from './widgets/familyManifest.ts';
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline & UsefulType;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS, FAMILY_VIEWS, liveFromBarrel(), fx());
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 10}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.entryPoints, ['ui/index.html']);
    assert.deepEqual(result.ui.deadProduction, [
      'ui/src/behindFixture.ts',
      'ui/src/deadSub.ts',
      'ui/src/runtimeLeak.ts',
      'ui/src/scratchOnly.ts',
    ]);
    assert.deepEqual(result.ui.productionTestImports, [
      { from: 'ui/src/main.tsx', to: 'ui/src/sneaky.fixture.tsx', role: 'fixture' },
    ]);
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
      import { runChart, shadow, useNs } from './consumer.ts';
      const Lazy = lazy(() => import('./lazyComp.ts'));
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(REQUEST_PATHS, Lazy, loadLazy, worker, used, ref, runChart(), shadow(1), useNs());
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
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
      '{"maxUnusedExports": 1}',
      'utf8',
    );
    writeFileSync(
      join(root, 'ui/src/main.tsx'),
      `
        import './styles.css';
        import type { SharedType } from './types.ts';
        import { REQUEST_PATHS } from './generated/dashboard.ts';
        import { live } from './mod.ts';
        const loadLazy = async () => (await import('./lazy.ts')).value;
        const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
        type Ref = import('./inlineType.ts').Inline;
        const used: SharedType = { product: 'live-product', head: 'live-head' };
        const ref: Ref = { ok: true };
        console.log(loadLazy, worker, used, ref, REQUEST_PATHS, live);
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
    assert.equal(headResult.totalIssues, 1);

    const compared = compareWithBase(headResult, baseResult, baseSha);
    assert.equal(compared.comparison, null);
    assert.match(compared.notice, new RegExp(baseSha.slice(0, 9)));

    rmSync(join(root, 'ui/src/inheritedOrphan.ts'));
    const cleanHead = await runDeadCodeCheck(root);
    assert.equal(cleanHead.totalIssues, 0);
    const cleanCompared = compareWithBase(cleanHead, baseResult, baseSha);
    assert.equal(cleanCompared.comparison, null);
    assert.match(cleanCompared.notice, /legacy-widgets-ratchet\.json/);

    const cmp = classifyAgainstBase(headResult, baseResult, baseSha);
    assert.deepEqual(cmp.ratchetRaises, []);
    assert.deepEqual(cmp.addedUnusedExports.map(e => e.key), ['ui/src/mod.ts:unusedAtHead']);
    assert.equal(cmp.deadProduction[0].inherited, true);
    assert.equal(cmp.newIssues, 1);
  });
});

test('R2-D3: does not seed uncalled functions in main.tsx and keeps side-effecting initializers live', async () => {
  await withRepo(baseFiles({
    'ui/src/onlyUncalled.ts': 'export const deadHelper = () => 1;',
    'ui/src/registrar.ts': 'export const register = (fn: () => number) => fn;',
    'ui/src/handler.ts': 'export const handler = () => 42;',
    'ui/src/feature.ts': `
      import { register } from './registrar.ts';
      import { handler } from './handler.ts';
      const off = register(handler);
      export const liveValue = 7;
    `,
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { deadHelper } from './onlyUncalled.ts';
      import { liveValue } from './feature.ts';
      function uncalledInMain() { return deadHelper(); }
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS, liveValue);
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.deadProduction, ['ui/src/onlyUncalled.ts']);
    assert.ok(result.ui.runtimeLive.has('ui/src/registrar.ts'));
    assert.ok(result.ui.runtimeLive.has('ui/src/handler.ts'));
    assert.deepEqual(result.exports.unusedExports, []);
  });
});

test('R2-D4: tracks computed string namespace access and exported dynamic-import loader member usage via compiler symbols', async () => {
  await withRepo(baseFiles({
    'ui/src/nsTarget.ts': 'export const draw = () => 1; export const unusedNs = () => 2;',
    'ui/src/chart.ts': 'export const draw = () => 3; export const unusedChart = () => 4;',
    'ui/src/chartLoader.ts': "export const loadChart = () => import('./chart.ts');",
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import * as ns from './nsTarget.ts';
      import { loadChart } from './chartLoader.ts';
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      const a = ns['draw']();
      async function run() {
        const mod = await loadChart();
        return mod.draw() + a;
      }
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS, run());
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.deadProduction, []);
    assert.deepEqual(
      result.exports.unusedExports.map(e => e.key),
      ['ui/src/chart.ts:unusedChart', 'ui/src/nsTarget.ts:unusedNs'],
    );
  });
});

test('R2-D6: base snapshot scans the same non-dashboard Rust caller paths as head (including examples)', async () => {
  await withRepo(baseFiles({
    'ui/src/generated/dashboard.ts': `
export const REQUEST_PATHS = {
  "live-product": {},
  "head:live-head": {},
  "tool-product": {},
  "crate-product": {},
  "example-product": {},
};
`,
    'crates/desktop/examples/tool.rs': 'fn main() { let _ = "example-product"; }',
  }), async root => {
    const git = args => execFileSync('git', args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] }).toString('utf8').trim();
    git(['init', '-q', '-b', 'main']);
    git(['add', '-A']);
    git(['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-m', 'base']);
    const baseSha = git(['rev-parse', 'HEAD']);

    const baseResult = await runCheckAtCommit(root, baseSha);
    assert.deepEqual(baseResult.rust.deadProducts, []);

    rmSync(join(root, 'crates/desktop/examples/tool.rs'));
    const headResult = await runDeadCodeCheck(root);
    assert.deepEqual(headResult.rust.deadProducts.map(p => p.product), ['example-product']);

    const cmp = classifyAgainstBase(headResult, baseResult, baseSha);
    assert.equal(cmp.newIssues, 1);
    assert.equal(cmp.inheritedIssues, 0);
    assert.equal(cmp.deadProducts[0].inherited, false);
  });
});

test('R2-D7: invalid explicit --against ref fails instead of silently skipping base comparison', async () => {
  await withRepo(baseFiles(), async root => {
    const git = args => execFileSync('git', args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] }).toString('utf8').trim();
    git(['init', '-q', '-b', 'main']);
    git(['add', '-A']);
    git(['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-m', 'base']);
    assert.throws(() => resolveBaseCommit(root, 'nonexistent-ref-xyz'), /Cannot resolve --against ref/);
    assert.throws(() => parseAgainstArg(['--against']), /--against requires a commit or ref/);
    assert.throws(() => parseAgainstArg(['--against=']), /--against requires a commit or ref/);
  });
});

test('R3-S18: uncalled nested functions inside a called function do not keep their imports or namespace reads live', async () => {
  await withRepo(baseFiles({
    'ui/src/nestedDead.ts': 'export const deadFromNested = () => 1;',
    'ui/src/nestedArrowDead.ts': 'export const deadFromArrow = () => 2;',
    'ui/src/nestedParamDead.ts': 'export const deadFromParam = () => 21;',
    'ui/src/nestedLive.ts': 'export const liveFromNested = () => 3;',
    'ui/src/nestedKeepDeadChild.ts': 'export const deadChildHelper = () => 4;',
    'ui/src/nestedNs.ts': 'export const usedNs = () => 5; export const deadNs = () => 6;',
    'ui/src/typeOnlyNs.ts': 'export const draw = () => 7; export const spare = () => 8;',
    'ui/src/typeOnlyDyn.ts': 'export const sideEffectValue = 9;',
    'ui/src/typeThenRuntimeHelper.ts': "import './typeThenRuntimeSideEffect.ts'; export const compute = () => 10;",
    'ui/src/typeThenRuntimeSideEffect.ts': 'console.log("side-effect");',
    'ui/src/shorthandTypeOnly.ts': 'export const draw = () => 11;',
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import { deadFromNested } from './nestedDead.ts';
      import { deadFromArrow } from './nestedArrowDead.ts';
      import { deadFromParam } from './nestedParamDead.ts';
      import { liveFromNested } from './nestedLive.ts';
      import { deadChildHelper } from './nestedKeepDeadChild.ts';
      import { compute } from './typeThenRuntimeHelper.ts';
      import { draw as shorthandDraw } from './shorthandTypeOnly.ts';
      import * as nestedNs from './nestedNs.ts';
      import * as typeOnlyNs from './typeOnlyNs.ts';
      function start(onTick = () => deadFromParam()) {
        const result = usedHoisted();
        function unusedInner() {
          return deadFromNested() + nestedNs.deadNs();
        }
        const unusedArrow = () => deadFromArrow();
        function usedHoisted() {
          return liveFromNested() + nestedNs.usedNs();
        }
        function keep() {
          function deadChild() {
            return deadChildHelper();
          }
          return 1;
        }
        type KeepType = typeof keep;
        const keepRef: KeepType = keep;
        function runBoth() {
          function inner() {
            return compute();
          }
          return inner();
        }
        type RunBothReturn = ReturnType<typeof runBoth>;
        const runBothVal: RunBothReturn = runBoth();
        function describeDirectShorthand() {
          return { shorthandDraw };
        }
        type DescC = ReturnType<typeof describeDirectShorthand>;
        function loadTypeOnly() {
          return typeOnlyNs.draw();
        }
        function dynTypeOnly() {
          return import('./typeOnlyDyn.ts');
        }
        type LoadedDraw = ReturnType<typeof loadTypeOnly>;
        type LoadedDyn = ReturnType<typeof dynTypeOnly>;
        const typed: [LoadedDraw | null, LoadedDyn | null, DescC | null] = [null, null, null];
        return result + keepRef() + runBothVal + typed.length;
      }
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(loadLazy, worker, used, ref, REQUEST_PATHS, start());
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.deadProduction, [
      'ui/src/nestedArrowDead.ts',
      'ui/src/nestedDead.ts',
      'ui/src/nestedKeepDeadChild.ts',
      'ui/src/nestedParamDead.ts',
    ]);
    assert.equal(result.ui.runtimeLive.has('ui/src/typeThenRuntimeSideEffect.ts'), true);
    assert.equal(result.ui.runtimeLive.has('ui/src/typeOnlyDyn.ts'), false);
    assert.deepEqual(result.exports.unusedExports.map(e => e.key), [
      'ui/src/nestedNs.ts:deadNs',
      'ui/src/typeOnlyDyn.ts:sideEffectValue',
      'ui/src/typeOnlyNs.ts:spare',
    ]);
  });
});

test('R3-S19: namespace passed to a local reader credits only consumed members, handles edge calls safely, and flags unused local export aliases', async () => {
  await withRepo(baseFiles({
    'ui/src/nsTarget.ts': 'export const draw = () => 1; export const spare = () => 2;',
    'ui/src/nsDestructured.ts': 'export const draw = () => 3; export const spare = () => 4;',
    'ui/src/nsEscaped.ts': 'export const a = () => 5; export const b = () => 6;',
    'ui/src/nsMethod.ts': 'export const draw = () => 7; export const spare = () => 8;',
    'ui/src/nsDefaultInit.ts': 'export const draw = () => 9; export const spare = () => 10;',
    'ui/src/nsThisParam.ts': 'export const draw = () => 11; export const spare = () => 12;',
    'ui/src/nsArgsObj.ts': 'export const draw = () => 13; export const spare = () => 14;',
    'ui/src/nsSameOffset.ts': 'export const draw = () => 15; export const spare = () => 16;',
    'ui/src/nsParenEscape.ts': 'export const a = () => 17; export const b = () => 18;',
    'ui/src/nsDocComment.ts': 'export const draw = () => 19; export const spare = () => 20;',
    'ui/src/nsPatternDefault.ts': 'export const draw = () => 21; export const spare = () => 22;',
    'ui/src/nsArgsShorthand.ts': 'export const draw = () => 23; export const spare = () => 24;',
    'ui/src/nsInnerOwnArgs.ts': 'export const draw = () => 25; export const spare = () => 26;',
    'ui/src/nsComputedKey.ts': 'export const key = "k"; export const spare = () => 27;',
    'ui/src/nsNestedOuterArgs.ts': 'export const draw = () => 28; export const spare = () => 29;',
    'ui/src/nsParamDestructDefault.ts': 'export const draw = () => 30; export const spare = () => 31;',
    'ui/src/nsAngleCast.ts': 'export const draw = () => 32; export const spare = () => 33;',
    'ui/src/angleReader.ts': 'export function readAngle(m: unknown) { return (<{ draw(): number }>m).draw(); }\n',
    'ui/src/helperA.ts': 'export function read(m: { draw(): number; spare(): number }) { return m.draw(); }\n',
    'ui/src/helperB.ts': 'export function read(m: { draw(): number; spare(): number }) { return m.spare(); }\n',
    'ui/src/aliasedMod.ts': `
      function draw() { return 10; }
      export const run = () => draw();
      export { draw as spare };
    `,
    'ui/src/aliasPair.ts': `
      function draw() { return 11; }
      export { draw, draw as spare };
    `,
    'ui/src/aliasBarrelSideEffect.ts': 'console.log("barrel-side-effect");',
    'ui/src/aliasBarrelSub.ts': `
      import './aliasBarrelSideEffect.ts';
      function draw() { return 12; }
      export { draw, draw as spare };
    `,
    'ui/src/aliasBarrel.ts': `export * from './aliasBarrelSub.ts';`,
    'ui/src/main.tsx': `
      import './styles.css';
      import type { SharedType } from './types.ts';
      import { REQUEST_PATHS } from './generated/dashboard.ts';
      import * as nsTarget from './nsTarget.ts';
      import * as nsDestructured from './nsDestructured.ts';
      import * as nsEscaped from './nsEscaped.ts';
      import * as nsMethod from './nsMethod.ts';
      import * as nsDefaultInit from './nsDefaultInit.ts';
      import * as nsThisParam from './nsThisParam.ts';
      import * as nsArgsObj from './nsArgsObj.ts';
      import * as nsSameOffset from './nsSameOffset.ts';
      import * as nsParenEscape from './nsParenEscape.ts';
      import * as nsDocComment from './nsDocComment.ts';
      import * as nsPatternDefault from './nsPatternDefault.ts';
      import * as nsArgsShorthand from './nsArgsShorthand.ts';
      import * as nsInnerOwnArgs from './nsInnerOwnArgs.ts';
      import * as nsComputedKey from './nsComputedKey.ts';
      import * as nsNestedOuterArgs from './nsNestedOuterArgs.ts';
      import * as nsParamDestructDefault from './nsParamDestructDefault.ts';
      import * as nsAngleCast from './nsAngleCast.ts';
      import * as aliasBarrelNs from './aliasBarrel.ts';
      import { readAngle } from './angleReader.ts';
      import { read as readA } from './helperA.ts';
      import { read as readB } from './helperB.ts';
      import { run } from './aliasedMod.ts';
      import { spare as importedAliasOnly } from './aliasPair.ts';
      function callDraw(m: typeof nsTarget) {
        function unusedNested() {
          return m.spare();
        }
        return m.draw();
      }
      function forwardDraw(m: typeof nsTarget) {
        return callDraw(m);
      }
      function callDestructured({ draw }: typeof nsDestructured) {
        return draw();
      }
      function escapeAll(m: typeof nsEscaped) {
        return Object.keys(m).length;
      }
      const api = { read(m: typeof nsMethod) { return m.spare(); } };
      api.read = m => m.draw();
      function readDefault(m: { draw(): number }, value = m.draw()) {
        return value;
      }
      function readPatternDefault(m: { draw(): number }, [v = m.draw()]: number[] = []) {
        return v;
      }
      function readParamDestructDefault(m: { draw(): number }, { draw } = m) {
        return draw();
      }
      function readComputedKey(m: { key: string }, { [m.key]: value }: Record<string, number> = {}) {
        return value ?? 0;
      }
      function readWithThis(this: unknown, m: { draw(): number }) {
        return m.draw();
      }
      function readArguments(_m: { draw(): number }) {
        return (arguments[0] as { draw(): number }).draw();
      }
      function readArgsShorthand(_m: { draw(): number }) {
        const saved = { arguments };
        return (saved.arguments[0] as { draw(): number }).draw();
      }
      function readNestedOuterArgs(_m: { draw(): number }) {
        const first = () => arguments[0] as { draw(): number };
        function draw() {
          return first().draw();
        }
        return draw();
      }
      function readWithInnerOwnArgs(m: { draw(): number }) {
        function inner(_x: number) {
          return arguments[0] as number;
        }
        return inner(m.draw());
      }
      function forwardBoth(m: typeof nsSameOffset) {
        return readA(m) + readB(m);
      }
      /** Reads draw. */
      const readDoc = (m: typeof nsDocComment) => m.draw();
      const loadLazy = async () => (await import('./lazy.ts')).value;
      const worker = new Worker(new URL('./lane.worker.ts', import.meta.url), { type: 'module' });
      type Ref = import('./inlineType.ts').Inline;
      const used: SharedType = { product: 'live-product', head: 'live-head' };
      const ref: Ref = { ok: true };
      console.log(
        loadLazy, worker, used, ref, REQUEST_PATHS,
        forwardDraw((nsTarget)), callDestructured(nsDestructured), escapeAll(nsEscaped),
        api.read(nsMethod), readDefault(nsDefaultInit), readPatternDefault(nsPatternDefault),
        readParamDestructDefault(nsParamDestructDefault), readAngle(nsAngleCast),
        readComputedKey(nsComputedKey), readWithThis(nsThisParam), readArguments(nsArgsObj),
        readArgsShorthand(nsArgsShorthand), readNestedOuterArgs(nsNestedOuterArgs),
        readWithInnerOwnArgs(nsInnerOwnArgs), forwardBoth(nsSameOffset),
        (nsParenEscape), readDoc(nsDocComment), run(), importedAliasOnly(), aliasBarrelNs.spare(),
      );
    `,
    'tools/legacy-widgets-ratchet.json': '{"maxUnusedExports": 0}',
  }), async root => {
    const result = await runDeadCodeCheck(root);
    assert.deepEqual(result.ui.deadProduction, []);
    assert.equal(result.ui.runtimeLive.has('ui/src/aliasBarrelSideEffect.ts'), true);
    assert.deepEqual(
      result.exports.unusedExports.map(e => e.key),
      [
        'ui/src/aliasBarrelSub.ts:draw',
        'ui/src/aliasPair.ts:draw',
        'ui/src/aliasedMod.ts:spare',
        'ui/src/nsAngleCast.ts:spare',
        'ui/src/nsComputedKey.ts:spare',
        'ui/src/nsDefaultInit.ts:spare',
        'ui/src/nsDestructured.ts:spare',
        'ui/src/nsDocComment.ts:spare',
        'ui/src/nsInnerOwnArgs.ts:spare',
        'ui/src/nsParamDestructDefault.ts:spare',
        'ui/src/nsPatternDefault.ts:spare',
        'ui/src/nsTarget.ts:spare',
        'ui/src/nsThisParam.ts:spare',
      ],
    );
  });
});
