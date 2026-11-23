import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { canvasPage } from '../../../tools/canvas-page.ts';
import { withNativePage } from '../../../tools/native-page.ts';
import type { PanelActions, PanelViewProps } from './RecorderPanel';
import type { RecorderSettings, Snapshot } from './status';

const settings: RecorderSettings = { recordEveryCampaign: true, autoSave: true, startWithWindows: false, startWithReplay: true, captureMode: 'full', liveCompression: 'low', autoCompress: true,
  campaigns: {}, effectiveSaveFolder: 'C:\\Recordings' };
const snapshot: Snapshot = { phase: 'waiting', campaign: null, recording: null, name: null, continued: false, hour: null, firstHour: null,
  savedBytes: null, spoolBytes: null, startedAt: null, stopping: false, problem: null, automatic: true, session: null, flag: null,
  captureMode: null, revision: 0 };

test('a panel clamped to a short screen scrolls its content and keeps its footer in reach', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  let markup: string;
  try {
    const { PanelView } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelView: (props: PanelViewProps) => unknown };
    markup = renderToStaticMarkup(createElement(PanelView as never, { snapshot, settings, view: 'settings', asking: false, error: null, now: 0,
      actions: new Proxy({}, { get: () => () => {} }) as PanelActions } as never));
  } finally { await vite.close(); }
  const styles = await Promise.all(['../styles.css', './recorder.css'].map(path => readFile(new URL(path, import.meta.url), 'utf8')));
  const { result } = await canvasPage(`
try {
  document.querySelector('canvas')?.remove();
  for (const css of ${JSON.stringify(styles)}) {
    const style = document.createElement('style');
    style.textContent = css;
    document.head.append(style);
  }
  // 1080p at 300%: the window is 360 CSS pixels wide and clamped to 320 tall.
  document.documentElement.style.cssText = 'width: 360px; height: 320px';
  const root = document.createElement('div');
  root.id = 'root';
  root.innerHTML = ${JSON.stringify(markup)};
  document.body.append(root);
  const footer = root.querySelector('.recorder-foot');
  const natural = root.querySelector('.recorder').getBoundingClientRect().height;
  const unscrolled = footer.getBoundingClientRect();
  root.scrollTop = root.scrollHeight;
  const sections = root.querySelectorAll('.recorder-section');
  const last = sections[sections.length - 1].getBoundingClientRect();
  const scrolled = footer.getBoundingClientRect();
  finish({ natural, viewport: root.clientHeight, unscrolledTop: unscrolled.top, unscrolledBottom: unscrolled.bottom,
    lastBottom: last.bottom, scrolledTop: scrolled.top, scrolledBottom: scrolled.bottom });
} catch (error) { finish({ error: String(error.stack || error) }); }
`);
  assert.equal(result.error, undefined, String(result.error));
  const at = result as Record<string, number>;
  assert.ok(at.natural > 320, `the settings run past the clamp (${at.natural})`);
  assert.equal(at.viewport, 320);
  assert.ok(at.unscrolledTop >= 0 && at.unscrolledBottom <= 320, `footer ${at.unscrolledTop}–${at.unscrolledBottom}`);
  assert.ok(at.scrolledTop >= 0 && at.scrolledBottom <= 320, `footer ${at.scrolledTop}–${at.scrolledBottom}`);
  assert.ok(at.lastBottom <= at.scrolledTop + 0.5, `last section ends ${at.lastBottom}, footer starts ${at.scrolledTop}`);
});

test('the panel reports a prompt seen only when the user acts inside it, never because it was drawn', async () => {
  await withNativePage('recorder-prompt', '/ui/src/recorder/RecorderPanel.dom.fixture.tsx', page => page.stage('recorder prompt', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.recorderPanelFixture.${expression}`);
    await act('render("main", 4)');
    assert.deepEqual(await act('seen()'), [], 'a Quit drawn in the panel isn’t one the user saw');
    await act('press(".recorder-head")');
    assert.deepEqual(await act('seen()'), [], 'a click elsewhere in the panel isn’t an act on the Quit');
    await act('press(".recorder-ask p")');
    assert.deepEqual(await act('seen()'), [4]);
    await act('render("main", 5, true)');
    await act('key(".recorder-prompt button")');
    assert.deepEqual(await act('seen()'), [4, 5], 'a key inside a prompt reissued for a new session reports that prompt');
  }, 60_000));
});

