import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { canvasPage } from '../../../tools/canvas-page.ts';
import { MIN_ZOOM, fitZoom } from './gameWindowScene.ts';
import type { Scene } from '../generated/dashboard';

/** A policy window's size in interface pixels, as the installed content lays it out. */
const WINDOW: [number, number] = [550, 500];
/** App viewports: the desktop app's smallest window, a short one, one too small for the window at
 * its smallest zoom, and a large screen. */
const VIEWPORTS: [string, number, number][] = [['app minimum', 1000, 650], ['short', 1000, 480], ['tiny', 380, 330], ['large', 3440, 1440]];
/** One opaque texel. */
const PIXEL = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==';

/** The scene's own markup at `zoom`, as `SceneView` renders it. */
async function sceneMarkup(zooms: number[]): Promise<string[]> {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { SceneView } = await vite.ssrLoadModule('/ui/src/components/GameWindow.tsx') as { SceneView: unknown };
    const scene: Scene = { name: 'synthetic_window', size: WINDOW, viewport: [1920, 1080], image: PIXEL,
      hovers: [{ rect: [10, 10, 200, 30], title: 'Policy', details: [] }], lists: [], notes: [] };
    return zooms.map(zoom => renderToStaticMarkup(createElement(SceneView as never, { scene, title: 'Policies', zoom,
      offsetOf: (_path: string, drawn: number) => drawn, onScroll: () => {} } as never)));
  } finally { await vite.close(); }
}

/** A window in its frame, each in a document of its own viewport: either a box of the given space
 * or the popover, sized by its own stylesheet from that viewport. Measures the frame and scene. */
async function measure(cases: { name: string; viewport: [number, number]; popover: boolean; space: [number, number]; markup: string }[]) {
  const css = ['./GameWindow.css', './GameWindowPopover.css'].map(path => readFile(new URL(path, import.meta.url), 'utf8'));
  const app = await readFile(new URL('../styles.css', import.meta.url), 'utf8');
  const srOnly = app.match(/\.sr-only \{[^}]*\}/)?.[0];
  assert.ok(srOnly, 'the app stylesheet declares .sr-only');
  const style = [...await Promise.all(css), srOnly].join('\n');
  const { result } = await canvasPage(`
try {
  const out = [];
  for (const space of ${JSON.stringify(cases)}) {
    const frame = document.createElement('iframe');
    frame.style.cssText = 'border:0;width:' + space.viewport[0] + 'px;height:' + space.viewport[1] + 'px';
    document.body.append(frame);
    const doc = frame.contentDocument;
    const window = '<div class="game-window" style="--game-window-width:${WINDOW[0]}px;--game-window-height:${WINDOW[1]}px">' + space.markup + '</div>';
    doc.open();
    doc.write('<!doctype html><style>body{margin:0}' + ${JSON.stringify(style)} + '</style>' + (space.popover
      ? '<div class="game-window-popover">' + window + '</div>'
      : '<div style="width:' + space.space[0] + 'px;height:' + space.space[1] + 'px">' + window + '</div>'));
    doc.close();
    const box = doc.querySelector('.game-window'), scene = doc.querySelector('.game-window-scene');
    const f = box.getBoundingClientRect(), s = scene.getBoundingClientRect();
    out.push({ name: space.name, left: s.left - f.left, top: s.top - f.top, width: s.width, height: s.height,
      scrollWidth: box.scrollWidth, scrollHeight: box.scrollHeight, clientWidth: box.clientWidth, clientHeight: box.clientHeight });
    frame.remove();
  }
  finish({ out });
} catch (error) { finish({ error: String(error.stack || error) }); }
`);
  assert.equal(result.error, undefined, String(result.error));
  return result.out as { name: string; left: number; top: number; width: number; height: number;
    scrollWidth: number; scrollHeight: number; clientWidth: number; clientHeight: number }[];
}

test('a game window fits, or scrolls from its start at its smallest zoom, in every space it is given', async () => {
  // Each viewport in a box of the space the popover gives it, and in the popover itself.
  const spaces = VIEWPORTS.map(([name, width, height]) => ({ name, viewport: [width, height] as [number, number],
    space: [Math.min(WINDOW[0], width - 44), Math.min(WINDOW[1], height - 92)] as [number, number] }));
  const cases = spaces.flatMap(space => [false, true].map(popover => ({ ...space, popover, name: `${space.name}${popover ? ' popover' : ''}`,
    zoom: fitZoom(WINDOW, space.space) })));
  const markup = await sceneMarkup(cases.map(space => space.zoom));
  const measured = await measure(cases.map((space, index) => ({ ...space, markup: markup[index] })));
  for (const [index, space] of measured.entries()) {
    const zoom = cases[index].zoom;
    assert.ok(zoom >= MIN_ZOOM, `${space.name}: never below the legible zoom`);
    // The popover's stylesheet gives a fitted window the same space its fit assumed (one that
    // scrolls loses its scrollbars' width).
    if (zoom > MIN_ZOOM)
      assert.deepEqual([space.clientWidth, space.clientHeight].map(Math.round), cases[index].space.map(Math.round), `${space.name}: the space the fit assumed`);
    assert.ok(space.left >= 0 && space.top >= 0, `${space.name}: the window's start is never cut off`);
    // Everything the window draws can be reached: within the frame, or by scrolling it.
    assert.ok(space.scrollWidth >= Math.floor(space.left + space.width) && space.scrollHeight >= Math.floor(space.top + space.height), space.name);
    const fits = zoom > MIN_ZOOM || (space.width <= space.clientWidth && space.height <= space.clientHeight);
    if (fits) assert.ok(space.scrollWidth <= space.clientWidth && space.scrollHeight <= space.clientHeight, `${space.name}: a fitted window does not scroll`);
    else assert.ok(space.scrollWidth > space.clientWidth || space.scrollHeight > space.clientHeight, `${space.name}: an unfitted window scrolls`);
  }
  const zooms = Object.fromEntries(cases.map(space => [space.name, space.zoom]));
  for (const name of ['app minimum', 'app minimum popover', 'large', 'large popover'])
    assert.equal(zooms[name], 1, `${name}: the window at its own size`);
  for (const name of ['short', 'short popover']) assert.ok(zooms[name] > MIN_ZOOM && zooms[name] < 1, `${name}: shrunk whole`);
  assert.equal(zooms.tiny, MIN_ZOOM);
});

test('production lines’ organisation trigger stays inside its cell and truncates its name at the widget’s minimum size', async () => {
  const css = await readFile(new URL('../widgets/production-lines/productionLines.css', import.meta.url), 'utf8');
  const { result } = await canvasPage(`
try {
  const style = document.createElement('style'); style.textContent = ${JSON.stringify(css)}; document.head.append(style);
  const out = [];
  for (const width of [48, 90, 240]) {
    const cell = document.createElement('span');
    cell.className = 'production-organisation';
    cell.style.width = width + 'px';
    cell.innerHTML = '<button type="button" class="production-factor production-organisation-trigger"><img alt="" style="width:22px;height:22px">'
      + '<small>Westmarch Heavy Industries and Tooling Works</small></button>';
    document.body.append(cell);
    const button = cell.firstElementChild, small = button.querySelector('small');
    const c = cell.getBoundingClientRect(), b = button.getBoundingClientRect();
    out.push({ width, left: b.left - c.left, right: c.right - b.right, truncated: small.scrollWidth > small.clientWidth });
    cell.remove();
  }
  finish({ out });
} catch (error) { finish({ error: String(error.stack || error) }); }
`);
  for (const cell of result.out as { width: number; left: number; right: number; truncated: boolean }[]) {
    assert.ok(cell.left >= 0 && cell.right >= -0.5, `${cell.width}px: the trigger stays inside its cell`);
    assert.ok(cell.truncated, `${cell.width}px: a long name is truncated, not overflowing`);
  }
});
