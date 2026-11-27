import assert from 'node:assert/strict';
import test from 'node:test';
import { build } from 'vite';

test('the shell and campaign controls leave Analytics renderers and game windows on demand', async () => {
  const chunks = new Map();
  await build({
    configFile: 'ui/vite.config.ts',
    logLevel: 'silent',
    build: { write: false },
    plugins: [{
      name: 'test-startup-assets',
      generateBundle(_options, bundle) {
        for (const chunk of Object.values(bundle)) {
          if (chunk.type === 'chunk') chunks.set(chunk.fileName, chunk);
        }
      },
    }],
  });

  const entry = [...chunks.values()].find(chunk => chunk.isEntry && chunk.name === 'main');
  const controls = [...chunks.values()].find(chunk => chunk.facadeModuleId?.endsWith('Dashboard.tsx'));
  assert.ok(entry, 'the desktop entry appears in the production graph');
  assert.ok(controls, 'the shared campaign controls appear in the graph');
  const modules = new Set();
  const seen = new Set();
  for (const chunk of [entry, controls]) {
    const pending = [chunk.fileName];
    while (pending.length) {
      const file = pending.pop();
      if (seen.has(file)) continue;
      seen.add(file);
      const imported = chunks.get(file);
      if (!imported) continue;
      for (const [id, module] of Object.entries(imported.modules)) {
        if (module.renderedLength > 0) modules.add(id.replaceAll('\\', '/').split('/ui/src/').at(-1));
      }
      pending.push(...imported.imports);
    }
  }
  const eager = [...modules].filter(id =>
    /^(?:dashboard\/(?:DashboardGrid|DashboardPages|WidgetPicker|EquipmentDesignerWindow|IndustrialOrganisationWindow|IntelligenceAgencyWindow)\.tsx|widgets\/(?:index\.ts|[^/]+\/(?:index\.tsx?|family\.tsx|[^/]+Projection\.ts))|families\/[^/]+\/hostAdapter\.ts)$/.test(id));
  assert.deepEqual(eager.sort(), []);
});
