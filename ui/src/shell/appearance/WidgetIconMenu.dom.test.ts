import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { withNativePage } from '../../../../tools/native-page.ts';
import type { fixture } from './WidgetIconMenu.dom.fixture';

type Snapshot = ReturnType<typeof fixture.snapshot>;

async function desktopImagePolicies() {
  const config = JSON.parse(await readFile(resolve('crates/desktop/tauri.conf.json'), 'utf8'));
  const security = config.app?.security ?? config.tauri?.security;
  return (['csp', 'devCsp'] as const).map(key => {
    const directive = String(security[key]).split(';').map(part => part.trim()).find(part => part.startsWith('img-src'));
    assert.ok(directive, `the desktop ${key} names an img-src`);
    return directive;
  });
}

test('a widget header icon picker opened by right-click returns focus to the widget on close', async context => {
  const policies = await desktopImagePolicies();
  let policy = policies[0];
  await withNativePage('icon-menu', '/ui/src/shell/appearance/WidgetIconMenu.dom.fixture.tsx', async page => {
    const act = <T = any,>(expression: string) => page.evaluate<T>(`window.iconFixture.${expression}`);
    const snapshot = () => act<Snapshot>('snapshot()');
    const escape = async () => {
      await page.press('Escape');
      await act('settle()');
    };
    await context.test('Escape from the icon picker opened on the header icon focuses the widget settings button', () =>
      page.stage('Icon menu Escape focus', async () => {
        await act('mount()'); await act('rightClickIcon()'); await act('choose("Change icon")');
        const open = await snapshot();
        assert.equal(open.pickerOpen, true);
        assert.equal(open.focusInPicker, true, 'the picker takes focus for its search');
        await escape();
        const closed = await snapshot();
        assert.equal(closed.pickerOpen, false);
        assert.equal(closed.active, 'settings', 'focus returns to the widget it was opened from, not the page');
      }, 60_000));
    await context.test('the colour picker opened the same way returns focus the same way', () =>
      page.stage('Colour picker Escape focus', async () => {
        await act('mount()'); await act('rightClickIcon()'); await act('choose("Change icon colour")');
        assert.equal((await snapshot()).pickerOpen, true);
        await escape();
        assert.equal((await snapshot()).active, 'settings');
      }, 60_000));
    await context.test('uploads become icons under the desktop app\'s image policy', async () => {
      for (const next of policies) {
        policy = next;
        await page.reload();
        await page.stage(`Icon uploads (${next})`, async () => {
          assert.deepEqual(await act('rasteriseUploads()'), [true, true], `a PNG and an untyped SVG under "${next}"`);
        }, 60_000);
      }
    });
    await context.test('closing never takes focus from somewhere the user moved it', () =>
      page.stage('Icon menu external focus', async () => {
        await act('mount()'); await act('rightClickIcon()'); await act('choose("Change icon")');
        await act('focusElsewhere()');
        assert.equal((await snapshot()).active, 'elsewhere');
        await escape();
        assert.equal((await snapshot()).active, 'elsewhere');
      }, 60_000));
  }, [{ name: 'icon-menu-csp', configureServer(server) {
    server.middlewares.use('/icon-menu-page', (_request, response, next) => {
      response.setHeader('Content-Security-Policy', policy);
      next();
    });
  } }]);
});
