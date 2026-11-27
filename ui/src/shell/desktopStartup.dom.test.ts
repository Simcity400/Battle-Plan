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

test('the recording library starts at first use and never presents its pending scan as empty', async () => {
  await withNativePage('startup-library', '/ui/src/shell/desktopStartup.dom.fixture.tsx', async page => {
    const { before, focused, waiting } = await page.evaluate('window.desktopStartup.openLibraryBeforePaint()');
    assert.equal(before.reads, 0);
    assert.match(before.text, /Reading your recordings folder/);
    assert.equal(focused.reads, 1);
    assert.equal(waiting.reads, 1);
    assert.match(waiting.text, /Reading your recordings folder/);
    const ready = await page.evaluate('window.desktopStartup.finishLibraryRead()');
    assert.match(ready.listed, /Example/);
    assert.equal(ready.reads, 1);
  });
});

test('a loaded campaign is not named the sole search match until the library resolves', async () => {
  await withNativePage('startup-library-search', '/ui/src/shell/desktopStartup.dom.fixture.tsx', async page => {
    const { before, after } = await page.evaluate('window.desktopStartup.searchWithAnOpenCampaign()');
    assert.equal(before.selected, 0);
    assert.equal(before.shortcuts, null);
    assert.match(before.status, /Reading your recordings folder/);
    assert.equal(after, 1);
  });
});

test('a campaign picker and folder field both show the pending library read', async () => {
  await withNativePage('startup-library-settings', '/ui/src/shell/desktopStartup.dom.fixture.tsx', async page => {
    const { before, after } = await page.evaluate('window.desktopStartup.readOtherLibrarySurfaces()');
    assert.equal(before.open, true);
    assert.equal(before.picker, 'Reading your recordings folder…');
    assert.equal(before.folder, before.picker);
    assert.equal(before.disabled, true);
    assert.match(after.picker, /Example/);
    assert.equal(after.folder, 'recordings');
    assert.equal(after.disabled, false);
  });
});
