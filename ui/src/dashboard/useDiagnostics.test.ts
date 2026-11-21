import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';

test('widget warnings reach the frame only when their text changes, and clear when their reporter leaves', async () => {
  const cells: { deps: readonly unknown[]; value?: unknown; cleanup?: unknown }[] = [];
  let cursor = 0, effects: (() => void)[] = [];
  const changed = (index: number, deps: readonly unknown[]) => !cells[index] || deps.some((value, i) => !Object.is(value, cells[index].deps[i]));
  const harness = {
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++;
      if (changed(index, deps)) cells[index] = { deps, value: make() };
      return cells[index].value;
    },
    useEffect(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++;
      if (!changed(index, deps)) return;
      const cleanup = cells[index]?.cleanup, cell: (typeof cells)[number] = { deps };
      cells[index] = cell;
      effects.push(() => { if (typeof cleanup === 'function') cleanup(); cell.cleanup = make(); });
    },
  };
  const globals = globalThis as typeof globalThis & { __diagnosticHooks?: typeof harness };
  globals.__diagnosticHooks = harness;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'diagnostic-hooks', enforce: 'pre', transform(source, id) {
      if (id.replaceAll('\\', '/').endsWith('/dashboard/useDiagnostics.ts'))
        return source.replace("import { useEffect, useMemo } from 'react';", 'const { useEffect, useMemo } = globalThis.__diagnosticHooks;');
    } }] });
  try {
    const { useDiagnostics } = await vite.ssrLoadModule('/ui/src/dashboard/useDiagnostics.ts') as typeof import('./useDiagnostics.ts');
    const frame: string[][] = [], panel: string[][] = [];
    const toFrame = (warnings: string[]) => { frame.push(warnings); }, toPanel = (warnings: string[]) => { panel.push(warnings); };
    const render = (handler: (warnings: string[]) => void, warnings: readonly string[] | null) => {
      cursor = 0;
      useDiagnostics(handler, warnings);
      const committed = effects; effects = [];
      for (const effect of committed) effect();
    };
    render(toFrame, ['Synthetic refusal']);
    render(toFrame, ['Synthetic refusal']);
    render(toFrame, []);
    render(toFrame, null);
    render(toFrame, []);
    assert.deepEqual(frame, [['Synthetic refusal'], [], []], 'an equal list is not resent; null leaves the frame to another view until it returns');
    render(toPanel, ['Synthetic refusal']);
    for (const cell of cells) if (typeof cell.cleanup === 'function') cell.cleanup();
    assert.deepEqual(frame, [['Synthetic refusal'], [], [], []], 'a new handler clears the old one');
    assert.deepEqual(panel, [['Synthetic refusal'], []], 'unmounting clears the warnings');
    assert.ok([...frame.slice(1), panel[1]].every(warnings => warnings === frame[1]), 'every empty report is one shared list');
  } finally { delete globals.__diagnosticHooks; await vite.close(); }
});
