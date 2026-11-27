import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { BUILT_IN_THEMES, DEFAULT_THEME_ID } from './appearance/themes.ts';

const canvas = BUILT_IN_THEMES.find(theme => theme.id === DEFAULT_THEME_ID)!.variants.dark!.canvas;
const file = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');

test('both entry pages paint the default dark canvas before loading scripts', () => {
  for (const page of ['../../index.html', '../../recorder.html']) {
    const html = file(page);
    const style = html.match(/<style>([\s\S]*?)<\/style>/)?.[1];
    assert.ok(style, `${page} has a first-paint style`);
    assert.match(style, new RegExp(`html\\s*\\{[^}]*background:\\s*${canvas}`));
    assert.match(style, /color-scheme:\s*dark/);
    assert.ok(html.indexOf('<style>') < html.indexOf('<script'));
  }
});

test('the main window starts hidden with the same canvas as its page', () => {
  const config = JSON.parse(file('../../../crates/desktop/tauri.conf.json'));
  const main = config.app.windows.find((window: { label: string }) => window.label === 'main');
  assert.equal(main.visible, false);
  assert.equal(main.backgroundColor, canvas);
});
