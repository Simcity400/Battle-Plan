import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage, type NativeKey } from '../../../tools/native-page.ts';
import type { fixture } from './Select.dom.fixture';

type Snapshot = ReturnType<typeof fixture.snapshot>;

test('mounted Select preserves authored choices, focus and label layout', async context => {
  await withNativePage('select', '/ui/src/components/Select.dom.fixture.tsx', async page => {
    const act = <T = any,>(expression: string) => page.evaluate<T>(`window.selectFixture.${expression}`);
    const snapshot = () => act<Snapshot>('snapshot()');
    const press = async (key: Extract<NativeKey, 'ArrowDown' | 'ArrowUp' | 'Home' | 'End' | 'Enter' | 'Tab'>, shift = false) => {
      await page.press(key, shift);
      await act('settle()');
    };
    await page.stage('Select DOM assertions', async () => {
      await context.test('an arriving catalogue preserves the active saved token and native Arrow/Enter', async () => {
        await act('mount(true)'); await act('open()');
        assert.equal((await snapshot()).activeLabel, 'Saved choice unavailable');
        await act('loadChoices()');
        const loaded = await snapshot();
        assert.equal(loaded.activeRole, 'option', 'the saved option retains keyboard focus when its index changes');
        assert.equal(loaded.activeValue, 'saved');
        assert.equal(loaded.activeLabel, 'Synthetic recorded choice 100 whose full name is meaningful');
        assert.ok(loaded.rows.length < 40, 'the catalogue remains virtual');
        await press('ArrowDown'); assert.equal((await snapshot()).activeValue, 'v101');
        await press('Enter'); assert.equal((await snapshot()).chosen, 'v101');
        assert.equal((await snapshot()).open, false);
      });
      await context.test('reordered active choices and unavailable choices reconcile by value', async () => {
        await act('mount()'); await act('open()'); await press('ArrowDown');
        await act('replaceChoices([...selectFixture.choices().filter(option => option.value !== "v101"), {value:"v101",label:"Moved active choice"}])');
        assert.equal((await snapshot()).activeValue, 'v101');
        assert.equal((await snapshot()).activeLabel, 'Moved active choice');
        await act('replaceChoices(selectFixture.choices().map(option => ({...option,disabled:option.value === "v101"})))');
        assert.equal((await snapshot()).activeValue, 'v0', 'a disabled active value falls back to the first enabled choice');
        await act('replaceChoices([{value:"disabled",label:"Disabled",disabled:true},{value:"enabled",label:"Enabled"}])');
        assert.equal((await snapshot()).activeValue, 'enabled', 'a removed active value falls back to the first enabled choice');
        await press('Enter'); assert.equal((await snapshot()).chosen, 'enabled');
        await act('mount()'); await act('open()');
        await act('replaceChoices([{value:"first",label:"First"},{value:"explained",label:"Explained",disabled:true,reason:"Needs a loaded campaign"},{value:"last",label:"Last"}])');
        await press('Home');
        assert.equal((await snapshot()).activeValue, 'first');
        await press('ArrowDown');
        assert.equal((await snapshot()).activeValue, 'explained', 'ArrowDown focuses a disabled option that explains why');
        await press('Enter');
        assert.equal((await snapshot()).chosen, null, 'Enter does not choose a disabled option');
        assert.equal((await snapshot()).open, true, 'menu stays open when pressing Enter on a disabled option');
        await press('ArrowDown');
        assert.equal((await snapshot()).activeValue, 'last');
        await press('Enter');
        assert.equal((await snapshot()).chosen, 'last');
      });
      await context.test('offscreen authored labels determine desktop width without horizontal scrolling', async () => {
        await act('mount(false, 5000)'); await act('open()');
        const sized = await snapshot(), long = sized.rows.find(row => row.index === 100)!;
        assert.ok(sized.menuWidth > 350, 'a full label supplies the menu intrinsic width');
        assert.ok(sized.rows.length < 40, '5,000 complete options stay windowed');
        assert.equal(sized.listScrollWidth, sized.listWidth, 'the menu has no horizontal overflow');
        assert.equal(long.labelWidth, long.labelScrollWidth, 'the full label is visible even when its tooltip contains a reason');
        await press('Home');
        assert.ok(!(await snapshot()).rows.some(row => row.index === 100), 'the widest label is offscreen');
        assert.equal((await snapshot()).menuWidth, sized.menuWidth, 'scrolling does not collapse the menu width');
        await press('End'); assert.equal((await snapshot()).activeValue, 'v4999');
      });
      await context.test('Tab and Shift+Tab leave the open select without choosing; arrows move between choices', async () => {
        await act('mount(false, 5000)'); await act('open()');
        const first = (await snapshot()).activeValue;
        await press('ArrowDown'); assert.notEqual((await snapshot()).activeValue, first, 'arrows move between choices');
        await press('Tab');
        const left = await snapshot();
        assert.equal(left.open, false, 'Tab closes the menu');
        assert.notEqual(left.activeRole, 'option', 'Tab moves on from the select');
        assert.equal(left.chosen, null, 'leaving does not choose');
        await act('open()'); await press('Tab', true);
        const back = await snapshot();
        assert.equal(back.open, false, 'Shift+Tab closes the menu');
        assert.notEqual(back.activeRole, 'option', 'Shift+Tab moves back from the select');
        assert.equal(back.chosen, null);
      });
      await context.test('narrow viewports wrap and measure long rows while preserving keyboard access', async () => {
        const viewport = new WebSocket(page.page);
        await new Promise<void>((done, reject) => {
          viewport.addEventListener('open', () => viewport.send(JSON.stringify({ id: 1, method: 'Emulation.setDeviceMetricsOverride',
            params: { width: 280, height: 700, deviceScaleFactor: 1, mobile: false } })));
          viewport.addEventListener('message', event => {
            const result = JSON.parse(String(event.data)); if (result.id !== 1) return;
            if (result.error) reject(new Error(JSON.stringify(result.error))); else done();
          });
          viewport.addEventListener('error', () => reject(new Error('Viewport CDP connection failed')));
        });
        try {
          await act('mount()'); await act('open()');
          const narrow = await snapshot(), long = narrow.rows.find(row => row.index === 100)!;
          assert.equal(narrow.viewportWidth, 280);
          assert.ok(narrow.menuLeft >= 11 && narrow.menuRight <= narrow.viewportWidth - 11);
          assert.equal(narrow.listScrollWidth, narrow.listWidth, 'narrow menus do not require horizontal scrolling');
          assert.ok(long.height > 32, 'wrapped text increases the measured row height');
          assert.equal(long.labelWidth, long.labelScrollWidth);
          assert.ok(long.top >= narrow.scrollTop! && long.top + long.height <= narrow.scrollTop! + narrow.listHeight!, 'the full focused row is revealed after measuring its wrapped text');
          assert.equal(narrow.rows.find(row => row.index === 101)!.top, long.top + long.height, 'the next row starts after the wrapped label');
          const same = await act<{ before: [string, number][]; after: [string, number][] }>('sameChoicesNow()');
          assert.deepEqual(same.after, same.before, 'an equal list passed anew keeps the wrapped rows where they were');
          await act('settle()');
          await press('ArrowDown'); assert.equal((await snapshot()).activeValue, 'v101');
          await press('End'); assert.equal((await snapshot()).activeValue, 'v149');
          await press('Enter'); assert.equal((await snapshot()).chosen, 'v149');
        } finally { viewport.close(); }
      });
    }, 60_000);
  });
});
