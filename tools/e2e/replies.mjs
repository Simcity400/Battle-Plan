import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createWriteStream, existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { createGzip } from 'node:zlib';

export const KIND = 'battleplan recorded replies';
export const VERSION = 4;
export const FORMAT_FILE = 'crates/replay-data/src/rows/registry.golden';
const TAKE_BUDGET = 1 << 20;

export function formatOf(text) {
  const lines = text.split(/\r?\n/).filter(line => line.length);
  return { lines, digest: createHash('sha256').update(lines.join('\n')).digest('hex') };
}

export const CAPTURE = `(() => {
  if (window.__replies) return true;
  const held = [];
  const lines = [];
  let cursor = 0;
  const timeline = () => document.querySelector('input.header-timeline');
  // The published clock while playing (the slider follows only anchors), the slider's value where no measured clock is.
  const head = () => { const hour = window.__perf?.hour(); if (hour != null) return hour; const input = timeline(); return input && input.value !== '' ? Number(input.value) : null; };
  const text = new TextDecoder();
  const COMMANDS = { dashboard_product: 'product', dashboard_open_source: 'open' };
  const fetched = window.fetch;
  window.fetch = function (input, init) {
    const promise = fetched.apply(this, arguments);
    const url = typeof input === 'string' ? input : input && input.url || '';
    const command = /^https?:\\/\\/ipc\\.localhost\\/([^/?]+)/.exec(url);
    const kind = command && COMMANDS[decodeURIComponent(command[1])];
    if (!kind) return promise;
    const at = performance.now(), hour = head();
    let args = null;
    try { args = typeof (init && init.body) === 'string' ? JSON.parse(init.body) : null; } catch { args = null; }
    promise.then(response => response.clone().arrayBuffer().then(bytes => {
      held.push({ t: kind, at, ms: performance.now() - at, head: hour, args, ok: response.headers.get('Tauri-Response') === 'ok', reply: text.decode(bytes) });
    })).catch(error => held.push({ t: kind, at, ms: performance.now() - at, head: hour, args, ok: false, reply: null, failed: String(error) }));
    return promise;
  };
  const followed = new BroadcastChannel('recorded-follow');
  let captureError;
  followed.onmessage = ({ data }) => {
    if (data.error) { captureError = data.error; return; }
    for (const message of data) held.push({ t: 'follow', at: performance.now(), head: head(), message });
  };
  const base64 = buffer => {
    const bytes = new Uint8Array(buffer);
    let out = '';
    const piece = 32768;
    for (let at = 0; at < bytes.length; at += piece) out += String.fromCharCode.apply(null, bytes.subarray(at, at + piece));
    return btoa(out);
  };
  window.__replies = {
    mark(entry) { held.push({ t: 'mark', at: performance.now(), head: head(), ...entry }); },
    take(budget) {
      if (captureError) throw new Error(captureError);
      const parts = [];
      let size = 0;
      while (size < budget) {
        if (!lines.length) {
          if (!held.length) break;
          const { message, ...entry } = held.shift();
          lines.push(JSON.stringify(message ? { ...entry, bytes: base64(message) } : entry));
          cursor = 0;
        }
        const piece = lines[0].slice(cursor, cursor + budget - size);
        cursor += piece.length;
        size += piece.length;
        const whole = cursor >= lines[0].length;
        parts.push([piece, whole]);
        if (whole) { lines.shift(); cursor = 0; }
      }
      return { parts, more: lines.length > 0 || held.length > 0 };
    },
  };
  return true;
})()`;

function capturePulls() {
  let followed;
  const read = Response.prototype.arrayBuffer;
  Response.prototype.arrayBuffer = function () {
    const body = read.call(this);
    if (this.url && new URL(this.url).pathname === '/follow' && this.ok) {
      body.then(buffer => {
        if (!buffer.byteLength) return;
        followed ??= new BroadcastChannel('recorded-follow');
        const view = new DataView(buffer), packets = [];
        for (let at = 0; at < view.byteLength;) {
          const size = view.getUint32(at, true);
          at += 4;
          if (at + size > view.byteLength) throw new Error('A recorded follow message ends early');
          packets.push(buffer.slice(at, at + size));
          at += size;
        }
        followed.postMessage(packets);
      }).catch(error => {
        followed ??= new BroadcastChannel('recorded-follow');
        followed.postMessage({ error: String(error) });
      });
    }
    return body;
  };
}

const LAYOUT = `(() => {
  const grid = document.querySelector('.dash-grid');
  const scroll = document.querySelector('.dash-grid-scroll');
  const box = element => { const r = element.getBoundingClientRect(); return { x: r.left, y: r.top, width: r.width, height: r.height }; };
  return { viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio }, grid: grid ? box(grid) : null, scroll: scroll ? box(scroll) : null,
    widgets: [...document.querySelectorAll('article.dash-widget')].map(a => ({ id: a.dataset.widget, kind: [...a.classList].find(c => c.startsWith('kind-'))?.slice(5) ?? null, ...box(a) })) };
})()`;

function selectBuildRecord(exe) {
  const dir = path.dirname(exe);
  const exeName = path.basename(exe).toLowerCase();
  const records = ['shell-build.json', 'boat-artifacts.json']
    .map(name => path.join(dir, name))
    .filter(existsSync);
  if (!records.length) {
    throw new Error(`no shell-build.json or boat-artifacts.json in ${dir}: the app's source revision is unknown, so its format cannot be named; pass --app-revision`);
  }
  if (records.length === 1) {
    return { record: records[0], data: JSON.parse(readFileSync(records[0], 'utf8')) };
  }
  const actualSha = createHash('sha256').update(readFileSync(exe)).digest('hex').toLowerCase();
  for (const record of records) {
    let data;
    try { data = JSON.parse(readFileSync(record, 'utf8')); } catch { continue; }
    const entry = Array.isArray(data?.artifacts)
      ? data.artifacts.find(item => typeof item?.path === 'string' && path.win32.basename(item.path).toLowerCase() === exeName)
      : null;
    if (typeof entry?.sha256 === 'string' && entry.sha256.toLowerCase() === actualSha) {
      return { record, data };
    }
  }
  throw new Error(`both shell-build.json and boat-artifacts.json exist in ${dir} and neither matches ${exe} SHA-256 ${actualSha}; pass --app-revision`);
}

export function appRevision(pid, named) {
  if (named) return named;
  const exe = execFileSync('powershell', ['-NoProfile', '-Command', `(Get-Process -Id ${Number(pid)}).Path`], { encoding: 'utf8' }).trim();
  const { record, data } = selectBuildRecord(exe);
  const { revision } = data;
  if (typeof revision !== 'string' || !revision) throw new Error(`${record} names no revision`);
  return revision;
}

export function formatAt(revision, root) {
  return formatOf(execFileSync('git', ['-C', root, 'show', `${revision}:${FORMAT_FILE}`], { encoding: 'utf8', maxBuffer: 64 << 20 }));
}

export class ReplyRecorder {
  static async start(app, file, { pid, revision, root }) {
    const app_revision = appRevision(pid, revision);
    const format = formatAt(app_revision, root);
    app.on('Inspector.targetCrashed', () => console.error('recording replies: the page crashed'));
    app.on('Inspector.detached', ({ reason }) => console.error(`recording replies: the debugger was detached: ${reason}`));
    app.on('Page.frameNavigated', ({ frame }) => { if (!frame.parentId) console.error(`recording replies: the page navigated to ${frame.url}; its capture is gone`); });
    await app.send('Inspector.enable');
    await app.evaluate(CAPTURE);
    const recorder = new ReplyRecorder(app, file);
    recorder.detach = app.on('Target.attachedToTarget', ({ sessionId, targetInfo }) => {
      const send = (method, params) => app.send(method, params, 60_000, sessionId);
      void (async () => {
        try {
          if (targetInfo.type === 'worker') {
            const result = await send('Runtime.evaluate', { expression: `(${capturePulls})()` });
            if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
          }
        } finally { await send('Runtime.runIfWaitingForDebugger', {}); }
      })().catch(error => { recorder.failed = error; });
    });
    await app.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true,
      filter: [{ type: 'worker' }, { exclude: true }] });
    await recorder.line({ kind: KIND, version: VERSION, formatDigest: format.digest, format: format.lines, app: { revision: app_revision }, recordedAt: new Date().toISOString() });
    return recorder;
  }

  constructor(app, file) {
    this.app = app;
    this.file = file;
    this.gzip = createGzip();
    this.closed = new Promise((resolve, reject) => {
      const out = createWriteStream(file);
      out.on('finish', resolve);
      out.on('error', reject);
      this.gzip.pipe(out);
    });
    this.counts = { open: 0, product: 0, follow: 0, mark: 0 };
    this.queue = Promise.resolve();
    this.partial = '';
    this.timer = setInterval(() => { void this.take().catch(() => {}); }, 1000);
  }

  line(entry) {
    return new Promise(resolve => { if (this.gzip.write(`${JSON.stringify(entry)}\n`)) resolve(); else this.gzip.once('drain', resolve); });
  }

  take() {
    const run = this.queue.then(async () => {
      if (this.failed) throw this.failed;
      for (let more = true; more;) {
        const taken = await this.app.evaluate(`window.__replies.take(${TAKE_BUDGET})`, 120_000);
        for (const [piece, whole] of taken.parts) {
          this.partial += piece;
          if (!whole) continue;
          const entry = JSON.parse(this.partial);
          this.partial = '';
          if (typeof entry.reply === 'string') {
            try { entry.reply = JSON.parse(entry.reply); } catch {   }
          }
          this.counts[entry.t] = (this.counts[entry.t] ?? 0) + 1;
          await this.line(entry);
        }
        more = taken.more;
      }
    });
    this.queue = run.catch(() => {});
    return run;
  }

  async mark(label, extra = {}) {
    const layout = await this.app.evaluate(LAYOUT);
    await this.app.evaluate(`window.__replies.mark(${JSON.stringify({ label, ...extra, layout })})`);
    await this.take();
  }

  async finish() {
    clearInterval(this.timer);
    await this.app.send('Target.setAutoAttach', { autoAttach: false, waitForDebuggerOnStart: false, flatten: true });
    this.detach();
    await this.take();
    this.gzip.end();
    await this.closed;
    return this.counts;
  }
}
