import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type MilitaryAirView from '../../dashboard/MilitaryAirView';

test('the merged air View adapts rendering, controls and new pins to the shared air view', async () => {
  type Props = Parameters<typeof MilitaryAirView>[0];
  const state = { view: undefined as Props | undefined };
  const globals = globalThis as typeof globalThis & { __airViewAdapter?: typeof state };
  globals.__airViewAdapter = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'air-view-adapter', enforce: 'pre',
      transform: (_source, id) => id.replaceAll('\\', '/').endsWith('/dashboard/MilitaryAirView.tsx')
        ? 'export default props => {globalThis.__airViewAdapter.view = props; return null;};' : undefined,
    }] });
  try {
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/air-kills-losses/AirKillsLossesWidget.tsx')).default;
    const pins: unknown[] = [], options: unknown[] = [];
    const props = { widget: { id: 'air', options: {} }, runtime: { addWidgetBelow: (_id: string, pin: unknown) => pins.push(pin) },
      active: true, onDiagnostics() {}, setOption: (key: string, value: unknown) => options.push([key, value]) } as unknown as WidgetProps;
    for (const saved of [{ show: 'breakdown', view: 'lines' }, { view: 'breakdown' }, { view: 'lines' }, { view: 'race' }]) {
      props.widget.options = saved as NonNullable<WidgetProps['widget']['options']>;
      renderToStaticMarkup(createElement(Widget, props));
      const view = state.view!;
      const breakdown = saved.show === 'breakdown' || saved.view === 'breakdown';
      assert.equal(view.widget.options?.show, breakdown ? 'breakdown' : 'main');
      assert.equal(view.widget.options?.view, breakdown ? 'race' : saved.view);
      assert.equal(view.family, 'airLosses');
      view.setOption!('show', 'breakdown');
      assert.deepEqual(options.at(-1), ['view', 'breakdown']);
      view.runtime.addWidgetBelow('air', { ...props.widget, time: { mode: 'follow' } as never, options: { show: 'breakdown', view: 'lines', causes: 'mod_cause' } });
      assert.deepEqual((pins.at(-1) as WidgetProps['widget']).options, { view: 'breakdown', causes: 'mod_cause' });
    }
  } finally { await vite.close(); delete globals.__airViewAdapter; }
});
