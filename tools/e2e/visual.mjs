import { copyFile, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { surfaceModel } from '../../ui/e2e/capture.mjs';
import { sleep } from './cdp.mjs';

const MODEL = `(${surfaceModel.toString()})`;

export class Visual {
  constructor(app, out, session) {
    this.app = app;
    this.session = session;
    this.directory = path.join(out, 'visual');
    this.items = [];
  }

  async init() { await mkdir(this.directory, { recursive: true }); }

  file(caseId) { return `${this.session}-${String(this.items.length + 1).padStart(4, '0')}-${caseId.replace(/[^\w.-]+/g, '_').slice(0, 100)}.png`; }

  async rest() {
    const at = await this.app.evaluate(`(() => { const g = document.querySelector('.dash-grid-scroll')?.getBoundingClientRect(); if (!g) return null;
      const empty = (x, y) => { const e = document.elementFromPoint(x, y); return e && !e.closest('article, [data-tooltip], button, a, input, [popover]'); };
      const last = ${JSON.stringify(this.resting ?? null)};
      if (last && empty(last.x, last.y)) return last;
      for (let y = g.bottom - 2; y > g.top; y -= 12) for (let x = g.right - 2; x > g.left; x -= 12) if (empty(x, y)) return { x, y };
      return null; })()`);
    this.resting = at;
    if (!at) throw new Error('no empty background to rest the pointer on');
    await this.app.mouse('mouseMoved', at.x, at.y);
  }

  paint() { return this.app.evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))'); }

  async capture(caseId, kind, selector = null) {
    try {
      await this.rest();
      await this.paint();
      const box = await this.app.evaluate(`(() => {
        const e = ${selector === null ? 'document.documentElement' : `document.querySelector(${JSON.stringify(selector)})`};
        if (!e) return null;
        const r = e.getBoundingClientRect();
        const left = Math.max(0, r.left), top = Math.max(0, r.top), right = Math.min(innerWidth, r.right), bottom = Math.min(innerHeight, r.bottom);
        if (right <= left || bottom <= top) return null;
        const model = e.matches('article.dash-widget') ? ${MODEL}(e) : { header: true, body: true, text: (e.innerText || '').slice(0, 20000) };
        return { x: left, y: top, width: right - left, height: bottom - top, model };
      })()`);
      if (!box) throw new Error(`nothing on screen for ${selector}`);
      const { data } = await this.app.send('Page.captureScreenshot', { format: 'png', clip: { x: box.x, y: box.y, width: box.width, height: box.height, scale: 1 } });
      const screenshot = this.file(caseId);
      await writeFile(path.join(this.directory, screenshot), Buffer.from(data, 'base64'));
      this.items.push({ kind, caseId, status: 'captured', model: { ...box.model, lists: [] }, screenshot });
    } catch (error) {
      this.items.push({ kind, caseId, status: 'error', error: error.message });
    }
  }

  async addFile(caseId, kind, source) {
    try {
      const screenshot = this.file(caseId);
      await copyFile(source, path.join(this.directory, screenshot));
      this.items.push({ kind, caseId, status: 'captured', model: { header: true, body: true, text: '', lists: [] }, screenshot });
    } catch (error) {
      this.items.push({ kind, caseId, status: 'error', error: error.message });
    }
  }

  fail(caseId, kind, error) { this.items.push({ kind, caseId, status: 'error', error }); }

  async finish(meta) {
    if (!this.items.length) return;
    await writeFile(path.join(this.directory, `manifest-${this.session}.json`), JSON.stringify({ complete: this.items.every(item => item.status === 'captured'), meta, items: this.items }));
  }
}

export { sleep };
