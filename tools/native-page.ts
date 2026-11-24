import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createReadStream, existsSync, rmSync } from 'node:fs';
import { createServer as createHttpServer, type ServerResponse } from 'node:http';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, extname, resolve, sep } from 'node:path';
import { setTimeout as pause } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { build, createServer, type PluginOption } from 'vite';
import { browserStage, ownedEdgeProcesses, stopBrowser, stopOwnedProfile } from './canvas-page.ts';

function cdp(endpoint: string, method: string, params: Record<string, unknown> = {}, signal?: AbortSignal) {
  signal?.throwIfAborted();
  return new Promise<any>((done, reject) => {
    const socket = new WebSocket(endpoint);
    const cleanup = () => { clearTimeout(timeout); signal?.removeEventListener('abort', abort); socket.close(); };
    const abort = () => { cleanup(); reject(signal!.reason); };
    const timeout = setTimeout(() => { cleanup(); reject(new Error(`${method} timed out`)); }, 30_000);
    signal?.addEventListener('abort', abort, { once: true });
    socket.addEventListener('open', () => socket.send(JSON.stringify({ id: 1, method, params })));
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data)); if (message.id !== 1) return;
      cleanup();
      if (message.error) reject(new Error(JSON.stringify(message.error))); else done(message.result);
    });
    socket.addEventListener('error', () => { cleanup(); reject(new Error(`${method} connection failed`)); });
  });
}

type ReplyFormat = { FORMAT_FILE: string; formatOf(text: string): { lines: string[]; digest: string } };

async function recordedReplies(path: string, response: ServerResponse): Promise<void> {
  const file = (process.env.BATTLEPLAN_RECORDED_REPLIES ?? process.env.HOI4_RECORDED_REPLIES) || null;
  if (path === '/format') {
    const { FORMAT_FILE, formatOf } = await import(new URL('./e2e/replies.mjs', import.meta.url).href) as ReplyFormat;
    const tree = formatOf(await readFile(fileURLToPath(new URL(`../${FORMAT_FILE}`, import.meta.url)), 'utf8'));
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify({ file: file && basename(file), formatFile: FORMAT_FILE, ...tree }));
    return;
  }
  if (!file || !existsSync(file)) {
    response.statusCode = 404;
    response.end(file ? `BATTLEPLAN_RECORDED_REPLIES names ${file}, which does not exist`
      : 'No recorded replies: set BATTLEPLAN_RECORDED_REPLIES to a file the acceptance sweep recorded (`--record-replies`) from the 13-year recording of the current format');
    return;
  }
  response.setHeader('Content-Type', 'application/gzip');
  createReadStream(file).pipe(response);
}

const KEY_CODES = { Tab: 9, Enter: 13, Escape: 27, PageUp: 33, PageDown: 34, End: 35, Home: 36, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40 };
export type NativeKey = keyof typeof KEY_CODES;

export interface NativePage {
  evaluate<T = any>(expression: string): Promise<T>;
  press(key: NativeKey, shift?: boolean): Promise<void>;
  page: string;
  stage<T>(label: string, run: (signal: AbortSignal) => Promise<T>, timeout?: number): Promise<T>;
}

const ROOT = fileURLToPath(new URL('../', import.meta.url));
const TYPES: Readonly<Record<string, string>> = { '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml',
  '.png': 'image/png', '.webp': 'image/webp', '.wasm': 'application/wasm', '.woff2': 'font/woff2' };
const builds = new Map<string, Promise<string>>();

function builtFixture(name: string, fixture: string, plugins: PluginOption[]): Promise<string> {
  const held = builds.get(fixture) ?? (async () => {
    const out = await mkdtemp(resolve(tmpdir(), `${name}-built-`));
    process.once('exit', () => rmSync(out, { recursive: true, force: true }));
    await build({ configFile: false, root: ROOT, logLevel: 'error', mode: 'development', plugins, define: { 'process.env.NODE_ENV': JSON.stringify('development') },
      build: { outDir: out, emptyOutDir: false, minify: false, sourcemap: false, modulePreload: false, target: 'esnext', reportCompressedSize: false, copyPublicDir: false,
        rollupOptions: { input: { fixture: resolve(ROOT, fixture.replace(/^\//, '')) }, preserveEntrySignatures: 'exports-only',
          output: { format: 'es', entryFileNames: 'fixture.js', chunkFileNames: 'chunks/[name]-[hash].js', assetFileNames: 'assets/[name]-[hash][extname]' } } } });
    return out;
  })();
  builds.set(fixture, held);
  return held;
}

async function builtServer(name: string, route: string, out: string) {
  const server = createHttpServer((request, response) => {
    const path = (request.url ?? '/').split('?')[0]!;
    if (path.startsWith('/recorded-replies')) {
      recordedReplies(path.slice('/recorded-replies'.length) || '/', response).catch(error => { response.statusCode = 500; response.end(String(error)); });
      return;
    }
    if (path === route) { response.setHeader('Content-Type', 'text/html'); response.end(`<!doctype html><title>${name}</title>`); return; }
    const file = resolve(out, `.${decodeURIComponent(path)}`);
    if (!file.startsWith(out + sep)) { response.statusCode = 403; response.end(); return; }
    stat(file).then(found => {
      if (!found.isFile()) throw new Error('not a file');
      response.setHeader('Content-Type', TYPES[extname(file)] ?? 'application/octet-stream');
      createReadStream(file).pipe(response);
    }, () => { response.statusCode = 404; response.end(`${path} is not in the built fixture`); });
  });
  await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  return { port: address.port, fixture: '/fixture.js', close: () => new Promise<void>(done => server.close(() => done())) };
}

async function devServer(name: string, route: string, fixture: string, profile: string, plugins: PluginOption[]) {
  const vite = await createServer({ configFile: false, root: ROOT,
    cacheDir: resolve(profile, 'vite'), optimizeDeps: { include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-dev-runtime', 'react-grid-layout'], noDiscovery: true },
    server: { host: '127.0.0.1', port: 0, watch: null, hmr: false, ws: false },
    plugins: [...plugins, { name: `${name}-page`, configureServer(server) {
      server.middlewares.use('/recorded-replies', (request, response) => {
        recordedReplies((request.url ?? '/').split('?')[0]!, response).catch(error => { response.statusCode = 500; response.end(String(error)); });
      });
      server.middlewares.use(route, (_request, response) => {
        response.setHeader('Content-Type', 'text/html'); response.end(`<!doctype html><title>${name}</title>`);
      });
    } }] });
  await vite.listen();
  const address = vite.httpServer!.address(); assert.ok(address && typeof address !== 'string');
  return { port: address.port, fixture, close: () => vite.close() };
}

export async function withNativePage(name: string, fixture: string, run: (page: NativePage) => Promise<void>, plugins: PluginOption[] = [], built = false): Promise<void> {
  const edge = (process.env.BATTLEPLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLEPLAN_CANVAS_BROWSER to its executable');
  const profile = await mkdtemp(resolve(tmpdir(), `${name}-`));
  const route = `/${name}-page`;
  const served = built ? await builtServer(name, route, await builtFixture(name, fixture, plugins)) : await devServer(name, route, fixture, profile, plugins);
  let browser: ReturnType<typeof spawn> | undefined, endpoint = '', browserPid: number | undefined;
  try {
    const address = { port: served.port };
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--edge-skip-compat-layer-relaunch',
      '--remote-debugging-port=0', `--user-data-dir=${profile}`], { windowsHide: true });
    browser.stdout?.resume(); browser.stderr?.resume();
    endpoint = await browserStage(browser, `${name} Edge startup`, async signal => {
      for (;;) {
        signal.throwIfAborted();
        try {
          const [port, path] = (await readFile(resolve(profile, 'DevToolsActivePort'), { encoding: 'utf8', signal })).trim().split('\n');
          if (port && path) return `ws://127.0.0.1:${port}/${path.replace(/^\//, '')}`;
        } catch (error) { signal.throwIfAborted(); if (!['ENOENT', 'EBUSY'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error; }
        await pause(50, undefined, { signal });
      }
    });
    const processes = await cdp(endpoint, 'SystemInfo.getProcessInfo');
    browserPid = processes.processInfo.find((entry: { type: string }) => entry.type === 'browser')?.id;
    assert.ok(browserPid && Number.isSafeInteger(browserPid));
    const { targetId } = await cdp(endpoint, 'Target.createTarget', { url: 'about:blank' });
    const targets = await fetch(`http://127.0.0.1:${new URL(endpoint).port}/json/list`).then(response => response.json()) as { id: string; webSocketDebuggerUrl: string }[];
    const target = targets.find(target => target.id === targetId)!.webSocketDebuggerUrl;
    const evaluate = async <T = any,>(expression: string): Promise<T> => {
      const result = await cdp(target, 'Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
      assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
      return result.result.value as T;
    };
    await cdp(target, 'Page.navigate', { url: `http://127.0.0.1:${address.port}${route}` });
    await browserStage(browser, `${name} page navigation`, async signal => {
      while (await evaluate('location.pathname') !== route) await pause(50, undefined, { signal });
    });
    await evaluate(`import(${JSON.stringify(served.fixture)}).then(() => true)`);
    const press = async (key: NativeKey, shift = false) => {
      await cdp(target, 'Input.dispatchKeyEvent', { type: 'keyDown', key, code: key, windowsVirtualKeyCode: KEY_CODES[key], modifiers: shift ? 8 : 0 });
      await cdp(target, 'Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, modifiers: shift ? 8 : 0 });
    };
    const owned = browser;
    const stage = <T,>(label: string, work: (signal: AbortSignal) => Promise<T>, timeout?: number) => browserStage(owned, label, work, timeout);
    await run({ evaluate, press, page: target, stage });
  } finally {
    try {
      if (browser) await stopBrowser(browser, async (_budget, signal) => { if (endpoint) await cdp(endpoint, 'Browser.close', {}, signal); }, async budget => {
        if (process.platform === 'win32') {
          await stopOwnedProfile(profile, browser!.exitCode !== null || browser!.signalCode !== null ? undefined : browser!.pid, browserPid, ownedEdgeProcesses, budget); return;
        }
        assert.ok(browserPid, 'Only the browser identified by this owned CDP endpoint may be stopped'); process.kill(browserPid, 'SIGKILL');
      }, 5_000, async (_budget, signal) => {
        if (process.platform === 'win32') return (await ownedEdgeProcesses(profile, browser!.exitCode !== null || browser!.signalCode !== null ? undefined : browser!.pid, false, undefined, signal)).length === 0;
        if (!browserPid) return browser!.exitCode !== null || browser!.signalCode !== null;
        try { process.kill(browserPid, 0); return false; } catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') return true; throw error; }
      }, { profile });
    } finally { await served.close(); }
    assert.ok(profile.startsWith(resolve(tmpdir()) + sep));
    await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
