import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { canvasPage } from '../../../tools/canvas-page.ts';
import { MIN_ZOOM, fitZoom } from './gameWindowScene.ts';

/** A policy window's size in interface pixels, as the installed content lays it out. */
const WINDOW: [number, number] = [550, 500];
/** App viewports: the desktop app's smallest window, a short one, one too small for the window at
 * its smallest zoom, and a large screen. */
const VIEWPORTS: [string, number, number][] = [['app minimum', 1000, 650], ['short', 1000, 480], ['tiny', 380, 330], ['large', 3440, 1440]];
/** The space the popover gives a window in a viewport (GameWindowPopover.css): the window's own
 * size, within the viewport less the popover's margins and heading. */
const SPACES = VIEWPORTS.map(([name, width, height]): [string, number, number] =>
  [name, Math.min(WINDOW[0], width - 44), Math.min(WINDOW[1], height - 92)]);

test('a game window fits, or scrolls from its start at its smallest zoom, in every space it is given', async () => {
  const css = await readFile(new URL('./GameWindow.css', import.meta.url), 'utf8');
  const cases = SPACES.map(([name, width, height]) => ({ name, width, height, zoom: fitZoom(WINDOW, [width, height]) }));
  const { result } = await canvasPage(`
try {
  const style = document.createElement('style'); style.textContent = ${JSON.stringify(css)}; document.head.append(style);
  const out = [];
  for (const space of ${JSON.stringify(cases)}) {
    const box = document.createElement('div');
    box.style.cssText = 'width:' + space.width + 'px;height:' + space.height + 'px';
    box.innerHTML = '<div class="game-window"><div class="game-window-scene" style="width:${WINDOW[0]}px;height:${WINDOW[1]}px;zoom:' + space.zoom + '"></div></div>';
    document.body.append(box);
    const frame = box.firstElementChild, scene = frame.firstElementChild;
    const f = frame.getBoundingClientRect(), s = scene.getBoundingClientRect();
    out.push({ name: space.name, zoom: space.zoom, left: s.left - f.left, top: s.top - f.top, width: s.width, height: s.height,
      scrollWidth: frame.scrollWidth, scrollHeight: frame.scrollHeight, clientWidth: frame.clientWidth, clientHeight: frame.clientHeight });
    box.remove();
  }
  finish({ out });
} catch (error) { finish({ error: String(error.stack || error) }); }
`);
  for (const space of result.out as { name: string; zoom: number; left: number; top: number; width: number; height: number;
    scrollWidth: number; scrollHeight: number; clientWidth: number; clientHeight: number }[]) {
    assert.ok(space.zoom >= MIN_ZOOM, `${space.name}: never below the legible zoom`);
    assert.ok(space.left >= 0 && space.top >= 0, `${space.name}: the window's start is never cut off`);
    // Everything the window draws can be reached: within the frame, or by scrolling it.
    assert.ok(space.scrollWidth >= Math.floor(space.left + space.width) && space.scrollHeight >= Math.floor(space.top + space.height), space.name);
    const fits = space.zoom > MIN_ZOOM || (space.width <= space.clientWidth && space.height <= space.clientHeight);
    if (fits) assert.ok(space.scrollWidth <= space.clientWidth && space.scrollHeight <= space.clientHeight, `${space.name}: a fitted window does not scroll`);
    else assert.ok(space.scrollWidth > space.clientWidth || space.scrollHeight > space.clientHeight, `${space.name}: an unfitted window scrolls`);
  }
  const zooms = Object.fromEntries((result.out as { name: string; zoom: number }[]).map(space => [space.name, space.zoom]));
  assert.equal(zooms['app minimum'], 1, 'the smallest app window shows the window at its own size');
  assert.equal(zooms.large, 1, 'a large screen shows it at its own size too');
  assert.ok(zooms.short > MIN_ZOOM && zooms.short < 1, 'a short window shrinks it whole');
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
