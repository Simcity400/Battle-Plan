import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage, type NativePage } from '../../../../tools/native-page.ts';
import type { Exactness, Placement } from './pixelExact.dom.fixture.tsx';

type Cdp = { send(method: string, params?: Record<string, unknown>): Promise<unknown>; close(): void };

const RATIOS = [1, 1.25, 1.5, 1.75, 2];
const FAMILIES = ['bars', 'breakdown', 'card', 'flow', 'heat-lanes', 'heat-matrix', 'meters', 'table', 'time-chart', 'timeline'];

async function cdpFor(page: NativePage): Promise<Cdp> {
  const { cdpSession } = await import(new URL('../../../../tools/e2e/cdp.mjs', import.meta.url).href) as { cdpSession(url: string): Promise<Cdp> };
  return cdpSession(page.page);
}

async function screenshot(cdp: Cdp): Promise<string> {
  return (await cdp.send('Page.captureScreenshot', { format: 'png' }) as { data: string }).data;
}

async function exactOnScreen(page: NativePage, cdp: Cdp, label: string): Promise<Exactness[]> {
  const settled = await page.evaluate<{ settled: boolean; pending: string[] }>('window.pixelExact.settle()');
  assert.ok(settled.settled, `${label}: every widget installed its picture (${settled.pending.join(', ')} pending)`);
  const shown = await screenshot(cdp);
  await page.evaluate('window.pixelExact.hideCanvases(true)');
  const under = await screenshot(cdp);
  await page.evaluate('window.pixelExact.hideCanvases(false)');
  const rows = await page.evaluate<Exactness[]>(`window.pixelExact.exactness(${JSON.stringify(shown)}, ${JSON.stringify(under)})`);
  assert.deepEqual(rows.map(row => row.id).sort(), FAMILIES, `${label}: one canvas per family`);
  for (const row of rows) {
    assert.deepEqual(row.backing, row.box, `${label}, ${row.id}: the backing ${row.backing.join('×')} is the canvas's device-pixel box ${row.box.join('×')}`);
    assert.deepEqual(row.backing, row.css.map(css => Math.round(css * row.dpr)), `${label}, ${row.id}: the backing is the CSS size × ${row.dpr}`);
    assert.ok(row.css.every((css, axis) => Math.abs(css * row.dpr - row.backing[axis]!) < 1e-3), `${label}, ${row.id}: the CSS size is whole device pixels`);
    assert.equal(row.differing, 0, `${label}, ${row.id}: ${row.differing} of ${row.compared} pixels on screen differ from the painter's bitmap by more than 1 level (worst ${row.worst}); canvas at device ${row.origin.map(v => v.toFixed(3)).join(', ')}; ${row.samples.join('; ')}`);
  }
  return rows;
}

for (const ratio of RATIOS) {
  test(`at device pixel ratio ${ratio}, every family's canvas shows the painter's pixels exactly: placed by the grid at fractional offsets, with the dashboard and a table scrolled by fractional amounts, after a resize and under a zoom`, async t => {
    await withNativePage(`pixel-exact-${String(ratio).replace('.', '-')}`, '/ui/src/families/host/pixelExact.dom.fixture.tsx', page => page.stage('pixel exact', async () => {
      const cdp = await cdpFor(page);
      try {
        await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
        const mounted = await page.evaluate<{ settled: boolean; pending: string[]; dpr: number }>('window.pixelExact.mount()');
        assert.equal(mounted.dpr, ratio, 'the browser runs at the device pixel ratio under test');
        assert.ok(mounted.settled, `every widget drew (${mounted.pending.join(', ')} pending)`);

        const placed = await exactOnScreen(page, cdp, 'placed');
        t.diagnostic(`placed: ${placed.map(row => `${row.id} ${row.backing.join('×')} at ${row.origin.map(v => v.toFixed(2)).join(',')}`).join('; ')}`);

        const dashboardTop = await page.evaluate<number>('window.pixelExact.scrollDashboard(37.3)');
        assert.ok(dashboardTop > 0, 'the dashboard scrolled');
        await exactOnScreen(page, cdp, `dashboard scrolled to ${dashboardTop}`);

        const tableTop = await page.evaluate<number>('window.pixelExact.scrollTable(13.3)');
        assert.ok(tableTop > 0, 'the table scrolled');
        await exactOnScreen(page, cdp, `table scrolled to ${tableTop}`);

        await page.evaluate('window.pixelExact.resize("breakdown", 5, 9, -7.4)');
        const resized = await exactOnScreen(page, cdp, 'resized');
        const before = placed.find(row => row.id === 'breakdown')!;
        assert.notDeepEqual(resized.find(row => row.id === 'breakdown')!.backing, before.backing, 'the resized widget has a new backing');

        await page.evaluate('window.pixelExact.zoom({ card: 1.1, bars: 0.9, table: 1.25 })');
        const zoomed = await exactOnScreen(page, cdp, 'zoomed');
        assert.ok(Math.abs(zoomed.find(row => row.id === 'card')!.dpr - ratio * 1.1) < 1e-6, 'a zoomed widget paints at its own device scale');

        if (ratio === 1) {
          await cdp.send('Emulation.setDeviceMetricsOverride', { width: 0, height: 0, deviceScaleFactor: 1.5, mobile: false });
          await page.evaluate('window.pixelExact.settle()');
          const moved = await page.evaluate<Placement[]>('window.pixelExact.placements()');
          for (const row of moved) {
            assert.ok(Math.abs(row.dpr / (zoomed.find(item => item.id === row.id)!.dpr) - 1.5) < 1e-6, `${row.id}: a new device pixel ratio reaches the painter`);
            assert.deepEqual(row.backing, row.css.map(css => Math.round(css * row.dpr)), `${row.id}: after the ratio changes, the backing is the CSS size × ${row.dpr}`);
          }
        }
      } finally {
        cdp.close();
      }
    }, 240_000), [], false, [`--force-device-scale-factor=${ratio}`, '--window-size=1440,1000']);
  });
}
