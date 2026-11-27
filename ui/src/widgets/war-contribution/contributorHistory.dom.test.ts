import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';
import { formatDateHover } from '../../families/valueFormat.ts';

test('installed War contribution rows open a cancellable history and keep its result through inspection and control changes', async () => {
  await withNativePage('war-contribution-history', '/ui/src/widgets/war-contribution/contributorHistory.dom.fixture.tsx', page => page.stage('installed contributor history', async () => {
    const act = <T,>(code: string) => page.evaluate<T>(`window.warHistoryFixture.${code}`);
    const { cdpSession } = await import(new URL('../../../../tools/e2e/cdp.mjs', import.meta.url).href);
    const cdp = await cdpSession(page.page);
    const mouse = async (selector: string, across: number, down: number, click = true) => {
      const box = await act<{ x: number; y: number; width: number }>(`box(${JSON.stringify(selector)})`);
      const x = box.x + box.width * across, y = box.y + down;
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
      if (click) {
        assert.equal(await act(`hitsBody(${x},${y})`), true, `widget receives pointer at ${x}, ${y}`);
        await act('nextFrames()');
        await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
        await act('nextFrames()');
        await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
      }
    };
    try {
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
      await act('mount("sides")');
      await act('activateTable("viewpoint:AAA")');
      await page.press('Enter');
      const lastHour = await act<number>('lastHour');
      const first = await act<{ calls: unknown; text: string; title: string }>('loading(1)');
      assert.deepEqual(first.calls, [[{ branchId: '1', viewpoint: 'AAA', warId: '0', throughHour: lastHour }, false]]);
      assert.match(first.text, /Reading contributor history/);
      assert.match(first.title, /AAA.*Fixture War/);
      await act('answer(0)');
      assert.deepEqual(await act<{ focus: string; rows: number; text: string }>('chart()').then(({ focus, rows }) => [focus, rows]), ['war-history-chart', 10]);
      await act('change(0,"casualties")');
      await act('change(1,"2")');
      await act('compare()');
      const partial = await act<{ text: string; spoken: string; count: number; snapshotHours: number[] }>('read()');
      assert.match(partial.text, /1 contributor not shown/);
      assert.match(partial.spoken, /partial.*not attributed/);
      assert.equal(partial.count, 1);
      assert.ok(partial.snapshotHours.every(hour => hour === lastHour));
      await page.press('Home');
      assert.match(await act<string>('tooltip()'), /AAA.*partial.*not attributed/);
      await page.press('ArrowRight');
      assert.match(await act<string>('tooltip()'), /Not recorded.*WarParticipation/);
      await page.press('End');
      await page.press('ArrowLeft');
      assert.ok(await act<{ caption: string }>('read()').then(reply => reply.caption.includes(formatDateHover(lastHour - 1))));
      assert.match(await act<string>('tooltip()'), /BBB.*partial.*not attributed/);
      assert.match(await act<{ spoken: string }>('read()').then(reply => reply.spoken), /EEE.*partial.*not attributed/);
      await mouse('.war-history-chart', 0.53, 66, false);
      assert.match(await act<string>('tooltip()'), /Recording paused in the fixture|partial.*not attributed|WarParticipation/);
      await page.press('Escape');
      assert.deepEqual(await act('closed()'), [true]);

      await act('activateTable("total:viewpoint")');
      await page.press('Enter');
      assert.match(await act<{ title: string }>('loading(2)').then(reply => reply.title), /^Contributor history/);
      await page.press('Escape');
      assert.deepEqual(await act('closed()'), [true, true]);
      await act('answer(1)');

      await act('mount("measure")');
      await mouse('.fh-body', 0.1, 18);
      await act('nextFrames()');
      assert.equal(await act<{ pick: string }>('read()').then(value => value.pick), 'equipmentDamage');
      await mouse('.fh-body', 0.47, 43);
      assert.match(await act<{ title: string }>('loading(3)').then(reply => reply.title), /AAA/);
      await mouse('.fh-body', 0.85, 43);
      assert.match(await act<{ title: string }>('loading(3,"GGG")').then(reply => reply.title), /GGG/);
      await act('answer(2,true)');
      await act('chart()');
      const missing = await act<{ text: string; count: number }>('read()');
      assert.match(missing.text, /Not recorded.*WarParticipation.*ledger was unavailable/);
      assert.equal(missing.count, 3);
      await page.press('Escape');
      assert.deepEqual(await act('closed()'), [true, true, true]);
    } finally {
      await cdp.close();
    }
  }, 45_000));
});
