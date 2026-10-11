import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { OPTIONS } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import type { PartsRaceViewConfig } from '../../charts/race/PartsRaceView';

test('fleet attribution keeps both saved native choices and zero HP does not claim no ships', async () => {
  const ownership = OPTIONS.find(field => field.key === 'ownership')!;
  assert.equal(ownership.label, 'Count fleets for');
  assert.equal(ownership.kind, 'choice');
  if (ownership.kind !== 'choice') throw new Error('Missing fleet choices');
  assert.deepEqual(ownership.choices, [{ value: 'roster', label: 'Country holding the fleet' }, { value: 'logical', label: 'Assigned country' }]);
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'fleet-config', enforce: 'pre',
      transform(_source, id) {
        if (id.replaceAll('\\', '/').endsWith('/charts/race/PartsRaceView.tsx')) return 'export default props => props.config;';
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/navy-size/NavySizeWidget.tsx');
    const config = Widget({}).props.config as PartsRaceViewConfig;
    for (const choice of ownership.choices) {
      const options = resolveOptions(OPTIONS, { ownership: choice.value });
      assert.equal(options.ownership, choice.value);
      assert.equal(config.ownership!(options), choice.label);
    }
    assert.equal(config.zeroMix!(resolveOptions(OPTIONS, { measure: 'ships' })), 'No ships at this time.');
    assert.equal(config.zeroMix!(resolveOptions(OPTIONS, { measure: 'strength' })), 'No hull strength at this time.');
  } finally { await vite.close(); }
});
