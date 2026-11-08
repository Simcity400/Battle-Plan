import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { writeFileSync } from 'node:fs';
const paint = page => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));

async function isolateImages(locator) {
  return locator.evaluate(async root => {
    const images = [...root.querySelectorAll('img')].filter(image => image.complete && image.naturalWidth > 0);
    for (const image of images) {
      const source = image.currentSrc || image.src;
      let url;
      if (source.startsWith('data:')) {
        // An unused media-type parameter changes only the resource identity.
        const comma = source.indexOf(','), header = source.slice(0, comma);
        const at = /;base64$/i.test(header) ? header.lastIndexOf(';') : header.length;
        const identity = [...crypto.getRandomValues(new Uint32Array(4))].map(value => value.toString(16)).join('-');
        url = `${header.slice(0, at)};parity-decode=${identity}${header.slice(at)}${source.slice(comma)}`;
      } else {
        const response = await fetch(source);
        if (!response.ok) throw new Error(`Cannot isolate image decode: ${source.slice(0, 160)} (${response.status})`);
        url = URL.createObjectURL(await response.blob());
      }
      image.removeAttribute('srcset'); image.removeAttribute('sizes'); image.src = url;
      await image.decode();
    }
  });
}

/** A widget's visible surface, evaluated in its page: whether it has a header and body, and its shown text. */
export function surfaceModel(root) {
  function shown(node) {
    if (getComputedStyle(node).visibility === 'hidden' || getComputedStyle(node).display === 'none') return false;
    const box = node.getBoundingClientRect();
    let left = Math.max(0, box.left), right = Math.min(innerWidth, box.right);
    let top = Math.max(0, box.top), bottom = Math.min(innerHeight, box.bottom);
    for (let ancestor = node.parentElement; ancestor; ancestor = ancestor.parentElement) {
      const style = getComputedStyle(ancestor), clip = ancestor.getBoundingClientRect();
      if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) { left = Math.max(left, clip.left); right = Math.min(right, clip.right); }
      if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) { top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom); }
    }
    return right > left && bottom > top;
  }
  const text = [], walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (!node.textContent.trim() || !node.parentElement || !shown(node.parentElement)) continue;
    const range = document.createRange(); range.selectNodeContents(node);
    if ([...range.getClientRects()].some(box => box.width > 0 && box.height > 0)) text.push(node.textContent);
  }
  return { header: !!root.querySelector('.dash-widget-header h2'),
    body: !!root.querySelector('.dash-widget-scaled')?.children.length,
    text: text.join('\n') };
}

const surfaceData = locator => locator.evaluate(surfaceModel);

export function modelError(list) {
  if (!list.complete || list.historyComplete === false || !Number.isSafeInteger(list.total) || list.total < 0
    || !Array.isArray(list.rows) || list.rows.length !== list.total) return 'Incomplete view model';
  const keys = new Set();
  for (let index = 0; index < list.rows.length; index++) {
    const row = list.rows[index], key = row?.key ?? row?.index;
    if (row?.index !== index || key == null || keys.has(String(key))) return `Invalid row identity or index at ${index}`;
    keys.add(String(key));
  }
  return null;
}

export async function captureWidget({ page, app, kind, directory, scenario, fileStem = kind, budgetMs = 60_000 }) {
  const started = performance.now();
  let timer;
  const check = () => { if (performance.now() - started >= budgetMs) throw Error('capture budget exceeded'); };
  const work = (async () => {
    await app.prepare(); check();
    await app.install(kind); check();
    const locator = app.widget(kind);
    await app.settle(locator); check();
    const action = await app.act?.(locator); check();
    if (action) { await app.settle(locator); check(); }
    const model = await surfaceData(locator);
    if (!model.header || !model.body) throw Error('Widget header or body missing');
    const screenshotFile = `${fileStem}.png`;
    // The fresh context owns these decode resources until its close. Playwright's
    // persistent screenshot session preserves its viewport and composed layers.
    await isolateImages(locator); check();
    await page.bringToFront(); await paint(page); check();
    await locator.screenshot({ path: path.join(directory, screenshotFile), scale: 'css', caret: 'initial' }); check();
    await app.prepareFullRows(locator, Math.max(1, budgetMs - (performance.now() - started))); check();
    model.lists = await app.fullRows(locator); check();
    for (const label of scenario?.models ?? []) if (!model.lists.some(list => list.label === label))
      throw Error(`Required exact model is missing: ${label}`);
    for (const list of model.lists) {
      const error = modelError(list);
      if (error) throw Error(`${list.label}: ${error}`);
    }
    if (await locator.locator('.windowed-rows,.log-scroll,.chart-table').count() && !model.lists.length)
      throw Error('Widget lacks an instrumented list view-model');
    if (app.errors.length) throw Error(JSON.stringify(app.errors));
    return { kind, ...(scenario ? { caseId: scenario.id, scenario } : {}),
      ...(action ? { action } : {}), status: 'captured', model, screenshot: screenshotFile };
  })();
  let capture;
  try {
    capture = await Promise.race([work, new Promise((_, reject) => {
      timer = setTimeout(() => { reject(Error('capture budget exceeded')); void page.close().catch(() => {}); }, budgetMs);
    })]);
    check();
  } catch (error) { capture = { kind, ...(scenario ? { caseId: scenario.id, scenario } : {}), status: 'error', error: String(error) }; }
  finally { clearTimeout(timer); }
  await mkdir(directory, { recursive: true });
  const bytes = JSON.stringify(capture);
  if (capture.status === 'captured' && performance.now() - started >= budgetMs) capture = { kind, ...(scenario ? { caseId: scenario.id, scenario } : {}), status: 'error', error: 'capture budget exceeded' };
  writeFileSync(path.join(directory, `${fileStem}.json`), capture.status === 'captured' ? bytes : JSON.stringify(capture));
  if (capture.status === 'captured' && performance.now() - started >= budgetMs) {
    capture = { kind, ...(scenario ? { caseId: scenario.id, scenario } : {}), status: 'error', error: 'capture budget exceeded' };
    writeFileSync(path.join(directory, `${fileStem}.json`), JSON.stringify(capture));
  }
  capture.wallMs = performance.now() - started;
  return capture;
}
