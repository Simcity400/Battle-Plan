import { readFile } from 'node:fs/promises';

export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export const log = (...args) => console.log(new Date().toISOString().slice(11, 23), ...args);

const PROBE = new URL('./probe.js', import.meta.url);
const APP_PAGE = /^http:\/\/tauri\.localhost\//;

export async function cdpSession(url) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = () => reject(new Error('CDP connection failed')); });
  let sequence = 0;
  const pending = new Map(), listeners = new Map();
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    if (message.id) {
      const item = pending.get(message.id);
      if (!item) return;
      pending.delete(message.id);
      clearTimeout(item.timer);
      if (message.error) item.reject(new Error(`${item.method}: ${JSON.stringify(message.error)}`)); else item.resolve(message.result);
      return;
    }
    for (const listener of listeners.get(message.method) ?? []) listener(message.params);
  };
  socket.onclose = ({ code, reason }) => {
    for (const item of pending.values()) item.reject(new Error(`CDP connection closed (${code}${reason ? ` ${reason}` : ''}) during ${item.method}`));
    pending.clear();
  };
  const on = (method, listener) => {
    if (!listeners.has(method)) listeners.set(method, new Set());
    listeners.get(method).add(listener);
    return () => { listeners.get(method)?.delete(listener); };
  };
  const send = (method, params = {}, timeout = 60_000, sessionId) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP ${method} timed out`)); }, timeout);
    pending.set(id, { resolve, reject, timer, method });
    socket.send(JSON.stringify({ id, method, params, sessionId }));
  });
  return { send, on, close: () => socket.close() };
}

export function createIpcTracker({ on, getResponseBody, now = Date.now, wait = sleep }) {
  const ipcCalls = new Map();
  const ipcById = new Map();
  const waiters = new Map();
  const parseIpcBody = text => {
    if (!text) return null;
    try { return JSON.parse(text); } catch { return text; }
  };
  const headerValue = (headers, name) => {
    if (!headers || typeof headers !== 'object') return null;
    const match = Object.entries(headers).find(([key]) => key.toLowerCase() === name);
    return match && typeof match[1] === 'string' ? match[1].trim().toLowerCase() : null;
  };
  on('Network.requestWillBeSent', ({ requestId, request }) => {
    const command = /^https?:\/\/ipc\.localhost\/([^/?]+)/.exec(request?.url ?? '')?.[1];
    if (command !== 'export_map') return;
    if ((request?.method ?? '').toUpperCase() !== 'POST') return;
    let resolveDone;
    const done = new Promise(resolve => { resolveDone = resolve; });
    const entry = { id: requestId, command, at: now(), settled: false, ok: null, status: null, tauriResponse: null, result: null, error: null, done, resolveDone };
    if (!ipcCalls.has(command)) ipcCalls.set(command, []);
    ipcCalls.get(command).push(entry);
    ipcById.set(requestId, entry);
    const pending = waiters.get(command);
    if (pending) {
      for (const listener of [...pending]) listener(entry);
    }
  });
  on('Network.responseReceived', ({ requestId, response }) => {
    const entry = ipcById.get(requestId);
    if (!entry) return;
    entry.status = response?.status ?? null;
    entry.tauriResponse = headerValue(response?.headers, 'tauri-response');
  });
  const finish = (entry, ok, result, error) => {
    entry.ok = ok;
    entry.result = ok ? result : null;
    entry.error = ok ? null : error;
    entry.settled = true;
    entry.resolveDone(entry);
  };
  on('Network.loadingFinished', ({ requestId }) => {
    const entry = ipcById.get(requestId);
    if (!entry || entry.settled) return;
    ipcById.delete(requestId);
    const statusOk = entry.status !== null && entry.status >= 200 && entry.status < 300;
    const ok = statusOk && entry.tauriResponse === 'ok';
    const fallbackError = entry.tauriResponse === 'error'
      ? 'Tauri command returned an error'
      : `HTTP ${entry.status ?? 'error'} (Tauri-Response: ${entry.tauriResponse ?? 'missing'})`;
    Promise.resolve(getResponseBody(requestId)).then(
      reply => {
        const body = reply?.body ?? '';
        const text = reply?.base64Encoded ? Buffer.from(body, 'base64').toString('utf8') : body;
        const parsed = parseIpcBody(text);
        const error = typeof parsed === 'string' && parsed ? parsed : fallbackError;
        finish(entry, ok, parsed, error);
      },
      () => finish(entry, ok, null, fallbackError),
    );
  });
  on('Network.loadingFailed', ({ requestId, errorText }) => {
    const entry = ipcById.get(requestId);
    if (!entry || entry.settled) return;
    ipcById.delete(requestId);
    finish(entry, false, null, errorText || 'IPC request failed');
  });
  const format = entry => ({ ok: entry.ok, status: entry.status, result: entry.result, error: entry.error });
  const waitForIpc = async (command, since, timeoutMs = 20_000) => {
    const deadline = now() + timeoutMs;
    const findMatch = () => (ipcCalls.get(command) ?? []).findLast(entry => entry.at >= since) ?? null;
    let match = findMatch();
    if (!match) {
      const remaining = deadline - now();
      if (remaining <= 0) return null;
      match = await new Promise(resolve => {
        if (!waiters.has(command)) waiters.set(command, new Set());
        let done = false;
        const cleanup = value => {
          if (done) return;
          done = true;
          waiters.get(command)?.delete(onEntry);
          resolve(value);
        };
        const onEntry = entry => { if (entry.at >= since) cleanup(entry); };
        waiters.get(command).add(onEntry);
        wait(remaining).then(() => cleanup(null));
      });
    }
    if (!match) return null;
    if (match.settled) return format(match);
    const remaining = deadline - now();
    if (remaining <= 0) {
      await Promise.resolve();
      return match.settled ? format(match) : null;
    }
    const finished = await Promise.race([match.done, wait(remaining).then(() => null)]);
    return finished ? format(finished) : null;
  };
  return { waitForIpc };
}

export async function connect(port, { timeoutMs = 120_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let target = null;
  while (!target) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(5000) })).json();
      target = targets.find(t => t.type === 'page' && APP_PAGE.test(t.url) && t.title === 'BattlePlan') ?? null;
    } catch {   }
    if (!target) {
      if (Date.now() > deadline) throw new Error(`no desktop app page on port ${port} within ${timeoutMs / 1000} s`);
      await sleep(500);
    }
  }
  const { send, on, close } = await cdpSession(target.webSocketDebuggerUrl);
  const evaluate = async (expression, timeout = 60_000) => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, timeout);
    if (result.exceptionDetails) throw new Error(`page: ${(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text).slice(0, 1500)}`);
    return result.result.value;
  };

  const messages = [];
  let current = 'startup';
  const note = (type, text, url) => messages.push({ at: Date.now(), during: current, type, text: String(text).slice(0, 800), url: url ?? null });
  on('Runtime.consoleAPICalled', ({ type, args, stackTrace }) => {
    if (type === 'error' || type === 'warning' || type === 'assert')
      note(type, args.map(a => a.value ?? a.description ?? '').join(' '), stackTrace?.callFrames?.[0]?.url);
  });
  on('Runtime.exceptionThrown', ({ exceptionDetails }) => note('exception', exceptionDetails.exception?.description ?? exceptionDetails.text, exceptionDetails.url));
  on('Log.entryAdded', ({ entry }) => {
    if ((entry.level === 'error' || entry.level === 'warning') && entry.source !== 'violation' && entry.source !== 'intervention')
      note(`log-${entry.level}`, entry.text, entry.url);
  });
  await send('Runtime.enable');
  await send('Log.enable');
  await send('Page.enable');
  const sent = new Map();
  on('Network.requestWillBeSent', ({ requestId, request }) => {
    const command = /^http:\/\/ipc\.localhost\/([^/?]+)/.exec(request.url)?.[1];
    if (command !== 'set_dashboard_maps') return;
    const read = request.postData !== undefined ? Promise.resolve(request.postData)
      : send('Network.getRequestPostData', { requestId }).then(reply => reply.postData, () => null);
    read.then(body => { if (body) sent.set(command, { at: Date.now(), args: JSON.parse(body) }); }, () => {});
  });
  const { waitForIpc } = createIpcTracker({
    on,
    getResponseBody: requestId => send('Network.getResponseBody', { requestId }),
  });
  await send('Network.enable', { maxPostDataSize: 1 << 20 });
  const native = await evaluate('!!window.__TAURI_INTERNALS__');
  if (!native) throw new Error('refusing to drive a page outside the desktop host');
  const probe = await readFile(PROBE, 'utf8');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: probe });
  await evaluate(`${probe};true`);

  const mouse = (type, x, y, extra = {}) => send('Input.dispatchMouseEvent', {
    type, x, y, button: type === 'mouseMoved' && !extra.buttons ? 'none' : 'left', clickCount: type === 'mouseMoved' ? 0 : 1, ...extra });
  const keyCodes = { Backspace: 8, Delete: 46, Enter: 13, Escape: 27, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Home: 36, End: 35, Tab: 9, PageUp: 33, PageDown: 34 };
  return {
    send, evaluate, on, messages,
    lastSent: command => sent.get(command) ?? null,
    waitForIpc,
    setCase(label) { current = label; },
    get case() { return current; },
    mouse,
    async click(x, y) { await mouse('mouseMoved', x, y); await mouse('mousePressed', x, y, { buttons: 1 }); await mouse('mouseReleased', x, y); },
    async key(key, modifiers = 0) {
      const code = keyCodes[key] ?? 0;
      await send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key, code: key, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
    },
    type: text => send('Input.insertText', { text }),
    async screenshot() { return Buffer.from((await send('Page.captureScreenshot', { format: 'png' })).data, 'base64'); },
    close,
  };
}
