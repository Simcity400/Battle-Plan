import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('table family plays across frames with zero React commits, zero layout reads on install, stable identity, keyboard mirror and narrow column chooser', async t => {
  await withNativePage('table-family', '/ui/src/families/table/tableFamily.dom.fixture.tsx', page => page.stage('table-family', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.tableFamilyDom.${expression}`);
    const mounted = await act<{ painted: boolean; initialRows: (string | null)[] }>('mount()');
    assert.equal(mounted.painted, true, 'initial table frame painted');
    assert.deepEqual(mounted.initialRows, ['GER', 'FRA', 'ITA', 'ENG'], 'missing ENG sorts last initially');

    const played = await act<{
      commits: number;
      totalInstallReads: number;
      installs: number;
      finalHour: number | null;
      mirrorRows: { key: string | null; cells: (string | null)[] }[];
    }>('playAndCount(12)');
    t.diagnostic(`12 playing steps: ${played.commits} React commits, ${played.totalInstallReads} install layout/style reads, ${played.installs} installs`);
    assert.equal(played.commits, 0, `zero React commits across 12 playing steps with unchanged structure: ${played.commits}`);
    assert.equal(played.totalInstallReads, 0, `zero layout/style reads during install: ${played.totalInstallReads}`);
    assert.ok(played.installs >= 10, `frames actually installed: ${played.installs}`);
    assert.equal(played.finalHour, 22);
    assert.equal(played.mirrorRows.at(-1)?.key, 'ENG', 'notRecorded row stays last');
    assert.ok(played.mirrorRows.at(-1)?.cells.some(c => c?.includes('Not recorded: ENG factories not recorded')), 'mirror speaks missing reason');

    const { cdpSession } = await import(new URL('../../../../tools/e2e/cdp.mjs', import.meta.url).href) as {
      cdpSession(url: string): Promise<{ send(method: string, params?: Record<string, unknown>): Promise<unknown>; close(): void }>;
    };
    const cdp = await cdpSession(page.page);
    try {
      const box = await act<{ left: number; top: number }>('bodyBox()');
      const px = Math.round(box.left + 40);
      const py = Math.round(box.top + 14);
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px, y: py });
      assert.equal(await act<boolean>('waitInspected()'), true, 'hover inspects row GER');

      const reordered = await act<(string | null)[]>('resortUnderPointer()');
      assert.equal(reordered[0], 'FRA', 'FRA moved into row 0 beneath the still pointer');

      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: px, y: py, button: 'left', clickCount: 1 });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: px + 1, y: py, button: 'left', clickCount: 1 });

      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: px, y: py, button: 'left', clickCount: 1 });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: px + 20, y: py, button: 'left', clickCount: 1 });

      const clicked = await act<string[]>('readActivated()');
      assert.deepEqual(clicked, ['GER'], 'click activates inspected row GER after re-sort, and >4px drag cancels');
    } finally {
      cdp.close();
    }

    const kbd = await act<{ afterDown: string | null; activated: string[]; liveStatus: string; moreText: string; menuOpen: boolean }>('testKeyboardAndNarrow()');
    assert.ok(kbd.afterDown !== null, 'ArrowDown moves focus to next mirror row');
    assert.equal(kbd.activated.length, 1, 'Enter activates focused row');
    assert.ok(kbd.moreText.includes('+'), `narrow table shows +N columns button: ${kbd.moreText}`);
    assert.equal(kbd.menuOpen, true, '+N columns button opens chooser');
  }));
});
