import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('a video export opened at startup waits visibly for its native preparation', async () => {
  await withNativePage('startup-export', '/ui/src/shell/desktopStartup.dom.fixture.tsx', async page => {
    const pending = await page.evaluate('window.desktopStartup.openExport()');
    assert.deepEqual(pending.asked, ['map_export_snapshot']);
    assert.match(pending.status, /Reading map camera and branch timeline/);
    assert.equal(pending.disabled, true);
    const prepared = await page.evaluate('window.desktopStartup.finishExportSetup()');
    assert.equal(prepared.disabled, false);
  });
});
