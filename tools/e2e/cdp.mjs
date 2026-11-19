// A CDP session on the desktop app's page: trusted input, screenshots and every console message.
import { readFile } from 'node:fs/promises';

export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export const log = (...args) => console.log(new Date().toISOString().slice(11, 23), ...args);

const PROBE = new URL('./probe.js', import.meta.url);
const APP_PAGE = /^http:\/\/tauri\.localhost\//;

/** A DevTools protocol session on one target's socket: `send` a command, `on` an event (it returns the unsubscribe). */
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

export async function connect(port, { timeoutMs = 120_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let target = null;
  while (!target) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(5000) })).json();
      // The main window, never a pop-out's page.
      target = targets.find(t => t.type === 'page' && APP_PAGE.test(t.url) && t.title === 'Battle Plan') ?? null;
    } catch { /* not listening yet */ }
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

  // Every console error and warning, exception and browser log error, with the case it happened in.
  const messages = [];
  let current = 'startup';
  const note = (type, text, url) => messages.push({ at: Date.now(), during: current, type, text: String(text).slice(0, 800), url: url ?? null });
  on('Runtime.consoleAPICalled', ({ type, args, stackTrace }) => {
    if (type === 'error' || type === 'warning' || type === 'assert')
      note(type, args.map(a => a.value ?? a.description ?? '').join(' '), stackTrace?.callFrames?.[0]?.url);
  });
  on('Runtime.exceptionThrown', ({ exceptionDetails }) => note('exception', exceptionDetails.exception?.description ?? exceptionDetails.text, exceptionDetails.url));
  // Chromium's own advice (violations, interventions) is not our code's output.
  on('Log.entryAdded', ({ entry }) => {
    if ((entry.level === 'error' || entry.level === 'warning') && entry.source !== 'violation' && entry.source !== 'intervention')
      note(`log-${entry.level}`, entry.text, entry.url);
  });
  await send('Runtime.enable');
  await send('Log.enable');
  await send('Page.enable');
  // The app's commands to its host go as requests to the IPC origin; the suite reads, never alters, them.
  const sent = new Map();
  on('Network.requestWillBeSent', ({ requestId, request }) => {
    const command = /^http:\/\/ipc\.localhost\/([^/?]+)/.exec(request.url)?.[1];
    if (command !== 'set_dashboard_maps') return;
    const read = request.postData !== undefined ? Promise.resolve(request.postData)
      : send('Network.getRequestPostData', { requestId }).then(reply => reply.postData, () => null);
    read.then(body => { if (body) sent.set(command, { at: Date.now(), args: JSON.parse(body) }); }, () => {});
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
    /** What the app last sent its host with `command`, as it sent it. */
    lastSent: command => sent.get(command) ?? null,
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
