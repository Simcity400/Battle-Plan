import assert from 'node:assert/strict';
import { AsyncLocalStorage } from 'node:async_hooks';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createReadStream, existsSync, rmSync } from 'node:fs';
import { createServer as createHttpServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, extname, resolve, sep } from 'node:path';
import type { Duplex } from 'node:stream';
import { setTimeout as pause } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { Agent, fetch } from 'undici';
import { build, createLogger, createServer, type PluginOption } from 'vite';
import { browserStage, ownedEdgeProcesses, stopBrowser, stopOwnedProfile } from './canvas-page.ts';
import { NativeCdp } from './native-cdp.ts';

export async function navigate(cdp: NativeCdp, url: string, signal: AbortSignal, ignoreCache = false) {
  const until = AbortSignal.any([signal, cdp.disconnected]);
  let finish!: () => void;
  const loadedPage = new Promise<void>(done => { finish = done; });
  const abort = () => finish();
  const loaded = new Set<string>();
  let loader: string | undefined;
  const off = cdp.onEvent(({ method, params }) => {
    if (method !== 'Page.lifecycleEvent' || params.name !== 'load') return;
    loaded.add(params.loaderId);
    if (params.loaderId === loader) finish();
  });
  until.addEventListener('abort', abort, { once: true });
  try {
    await cdp.send('Page.enable', {}, until);
    await cdp.send('Page.setLifecycleEventsEnabled', { enabled: true }, until);
    if (ignoreCache) {
      await cdp.send('Network.enable', {}, until);
      await cdp.send('Network.setCacheDisabled', { cacheDisabled: true }, until);
    }
    const result = await cdp.send('Page.navigate', { url }, until);
    if (result.errorText) throw new Error(`Page.navigate to ${url}: ${result.errorText}`);
    loader = result.loaderId;
    if (loader === undefined || !loaded.has(loader)) await loadedPage;
    until.throwIfAborted();
    if (ignoreCache) await cdp.send('Network.setCacheDisabled', { cacheDisabled: false }, until);
  } finally {
    until.removeEventListener('abort', abort);
    off();
  }
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
  reload(): Promise<void>;
  page: string;
  stage<T>(label: string, run: (signal: AbortSignal) => Promise<T>, timeout?: number): Promise<T>;
}

export interface ServerFailure {
  url: string;
  status?: number;
  reason: string;
}

export interface BrowserNetworkFailure {
  url: string;
  status?: number;
  reason: string;
}

export interface ServedFixture {
  port: number;
  fixture: string;
  failures: ServerFailure[];
  ready(signal?: AbortSignal): Promise<void>;
  close(): Promise<void>;
}

function errorDetail(error: unknown): string {
  if (!(error instanceof Error)) return String(error);
  const parts = [error.message];
  let current: unknown = error.cause;
  while (current instanceof Error) {
    const code = (current as NodeJS.ErrnoException).code;
    parts.push(code ? `${code}: ${current.message}` : current.message);
    current = current.cause;
  }
  return parts.join(' — ');
}

function captureBodyChunk(chunk: unknown, encoding?: BufferEncoding): Buffer | null {
  if (chunk == null || typeof chunk === 'function') return null;
  if (Buffer.isBuffer(chunk)) return chunk;
  if (ArrayBuffer.isView(chunk)) return Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength);
  if (typeof chunk === 'string') return Buffer.from(chunk, encoding);
  return null;
}

function handleClientError(record: (url: string, status: number | undefined, reason: string) => void) {
  return (error: Error, socket: Duplex) => {
    record('<socket>', undefined, `HTTP client error: ${error.message}`);
    if (socket.writable && !socket.destroyed) {
      socket.end('HTTP/1.1 400 Bad Request\r\n\r\n');
    } else {
      socket.destroy();
    }
  };
}

export function formatFixtureLoadError(
  fixture: string,
  detail: string,
  serverFailures: readonly ServerFailure[],
  networkFailures: readonly BrowserNetworkFailure[],
): Error {
  const parts: string[] = [];
  for (const failure of serverFailures) {
    const status = failure.status === undefined ? '' : `${failure.status} `;
    parts.push(`server ${status}${failure.url}: ${failure.reason}`);
  }
  for (const failure of networkFailures) {
    const status = failure.status === undefined ? '' : `HTTP ${failure.status} `;
    parts.push(`browser ${status}${failure.reason} (${failure.url})`);
  }
  if (detail) parts.push(detail);
  return new Error(`fixture could not be loaded (${fixture}): ${parts.join('; ')}`);
}

const JS_MIME_TYPES = new Set([
  'application/ecmascript',
  'application/javascript',
  'application/x-ecmascript',
  'application/x-javascript',
  'text/ecmascript',
  'text/javascript',
  'text/javascript1.0',
  'text/javascript1.1',
  'text/javascript1.2',
  'text/javascript1.3',
  'text/javascript1.4',
  'text/javascript1.5',
  'text/jscript',
  'text/livescript',
  'text/x-ecmascript',
  'text/x-javascript',
]);

async function withHttpProbe<T>(work: (dispatcher: Agent) => Promise<T>): Promise<T> {
  const dispatcher = new Agent({ connections: 1 });
  try {
    return await work(dispatcher);
  } finally {
    await dispatcher.destroy();
  }
}

async function fetchFixtureModule(
  target: string,
  fixture: string,
  failures: ServerFailure[],
  dispatcher: Agent,
  signal?: AbortSignal,
): Promise<{ code: string; url: string }> {
  const before = failures.length;
  let response: Awaited<ReturnType<typeof fetch>>;
  let text: string;
  try {
    response = await fetch(target, { dispatcher, signal });
    if (!response.ok) {
      const body = (await response.text().catch(() => '')).replace(/\s+/g, ' ').trim().slice(0, 512);
      const serverSeen = failures.slice(before);
      const detail = serverSeen.length > 0 ? '' : `server ${response.status} ${target}: ${body || response.statusText || 'HTTP error'}`;
      throw formatFixtureLoadError(fixture, detail, serverSeen, []);
    }
    text = await response.text();
  } catch (error) {
    if (error instanceof Error && error.message.startsWith('fixture could not be loaded (')) throw error;
    if (signal?.aborted) throw signal.reason;
    throw formatFixtureLoadError(fixture, `HTTP probe failed for ${target}: ${errorDetail(error)}`, failures.slice(before), []);
  }
  const contentType = response.headers.get('content-type') ?? '';
  const mimeType = contentType.split(';', 1)[0]!.trim().toLowerCase();
  if (!JS_MIME_TYPES.has(mimeType)) {
    throw formatFixtureLoadError(fixture, `server returned non-JavaScript content-type ${JSON.stringify(contentType)} for ${response.url || target}`, failures.slice(before), []);
  }
  return { code: text, url: response.url || target };
}

export async function runFixtureLoadStage(
  stage: <T>(label: string, work: (signal: AbortSignal) => Promise<T>, timeout?: number) => Promise<T>,
  label: string,
  served: Pick<ServedFixture, 'fixture' | 'failures'>,
  work: (signal: AbortSignal, networkFailures: BrowserNetworkFailure[]) => Promise<void>,
  timeout?: number,
): Promise<void> {
  const networkFailures: BrowserNetworkFailure[] = [];
  try {
    await stage(label, signal => work(signal, networkFailures), timeout);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith('fixture could not be loaded (')) throw error;
    throw formatFixtureLoadError(served.fixture, errorDetail(error), served.failures, networkFailures);
  }
}

export async function loadFixture(
  cdp: NativeCdp,
  served: Pick<ServedFixture, 'fixture' | 'failures'>,
  signal: AbortSignal,
  networkFailures: BrowserNetworkFailure[] = [],
): Promise<void> {
  const failuresBefore = served.failures.length;
  const requestUrls = new Map<string, string>();
  const httpFailedUrls = new Set<string>();
  const off = cdp.onEvent(({ method, params }) => {
    if (method === 'Network.requestWillBeSent') {
      requestUrls.set(params.requestId, params.request.url);
    } else if (method === 'Network.responseReceived') {
      const { url, status, statusText } = params.response;
      if (status >= 400 && !url.endsWith('/favicon.ico')) {
        httpFailedUrls.add(url);
        networkFailures.push({ url, status, reason: statusText || 'HTTP error' });
      }
    } else if (method === 'Network.loadingFailed') {
      const url = requestUrls.get(params.requestId) ?? '<unknown>';
      requestUrls.delete(params.requestId);
      if (!url.endsWith('/favicon.ico') && !(params.canceled && httpFailedUrls.has(url))) {
        networkFailures.push({ url, reason: params.errorText || 'loading failed' });
      }
    } else if (method === 'Network.loadingFinished') {
      requestUrls.delete(params.requestId);
    }
  });
  try {
    await cdp.send('Network.enable', {}, signal);
    const result = await cdp.send('Runtime.evaluate', {
      expression: `import(${JSON.stringify(served.fixture)}).then(() => true)`, awaitPromise: true, returnByValue: true,
    }, signal);
    const exception = result?.exceptionDetails;
    if (exception) throw new Error(exception.exception?.description ?? exception.text ?? JSON.stringify(exception));
  } catch (error) {
    throw formatFixtureLoadError(served.fixture, errorDetail(error), served.failures.slice(failuresBefore), networkFailures);
  } finally {
    off();
  }
}

function staticImportsOf(code: string, fromUrl: string): string[] {
  let ast: ReturnType<typeof parseAst>;
  try {
    ast = parseAst(code, { lang: 'js' }, fromUrl);
  } catch {
    return [];
  }
  const imports: string[] = [];
  for (const stmt of ast.body) {
    if ((stmt.type === 'ImportDeclaration' || stmt.type === 'ExportNamedDeclaration' || stmt.type === 'ExportAllDeclaration') && typeof stmt.source?.value === 'string') {
      const spec = stmt.source.value;
      if (spec.startsWith('/') || spec.startsWith('./') || spec.startsWith('../') || spec.startsWith('http://') || spec.startsWith('https://')) {
        imports.push(new URL(spec, fromUrl).href);
      }
    }
  }
  return imports;
}

const ROOT = fileURLToPath(new URL('../', import.meta.url));
const PAGE_LOAD_MS = 180_000;
const TYPES: Readonly<Record<string, string>> = { '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml',
  '.png': 'image/png', '.webp': 'image/webp', '.wasm': 'application/wasm', '.woff2': 'font/woff2' };
const builds = new Map<string, Promise<string>>();

function fixtureClose(server: Server, record: (url: string, status: number | undefined, reason: string) => void,
  dispose?: () => Promise<void>) {
  const sockets = new Map<Duplex, Promise<void>>();
  server.on('connection', socket => {
    sockets.set(socket, new Promise<void>(done => socket.once('close', () => { sockets.delete(socket); done(); })));
  });
  server.keepAliveTimeout = PAGE_LOAD_MS;
  server.headersTimeout = PAGE_LOAD_MS + 1_000;
  server.on('clientError', handleClientError(record));
  let closing: Promise<void> | undefined;
  return () => closing ??= (async () => {
    const listenerClosed = server.listening ? once(server, 'close') : undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const disposed = dispose && Promise.race([
        dispose(),
        new Promise<void>(done => { timer = setTimeout(done, 500); }),
      ]);
      if (server.listening) server.close();
      const closed = [...sockets].map(([socket, done]) => { socket.destroy(); return done; });
      const results = await Promise.allSettled([listenerClosed, ...closed, disposed]);
      assert.equal(server.listening, false, 'fixture listener still open after teardown');
      assert.equal(sockets.size, 0, 'fixture connections still open after teardown');
      for (const result of results) {
        if (result.status === 'rejected') throw result.reason;
      }
    } finally {
      clearTimeout(timer);
    }
  })();
}

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

export async function builtServer(name: string, route: string, out: string): Promise<ServedFixture> {
  const failures: ServerFailure[] = [];
  const record = (url: string, status: number | undefined, reason: string) => {
    if (url !== '/favicon.ico') failures.push({ url, status, reason });
  };
  const server = createHttpServer((request, response) => {
    const url = request.url ?? '/';
    const path = url.split('?')[0]!;
    if (path === '/favicon.ico') { response.statusCode = 204; response.end(); return; }
    if (path.startsWith('/recorded-replies')) {
      recordedReplies(path.slice('/recorded-replies'.length) || '/', response).catch(error => {
        const reason = error instanceof Error ? error.message : String(error);
        record(url, 500, reason);
        response.statusCode = 500;
        response.end(reason);
      });
      return;
    }
    if (path === route) {
      const html = `<!doctype html><title>${name}</title>`;
      response.setHeader('Content-Type', 'text/html');
      response.setHeader('Content-Length', Buffer.byteLength(html));
      response.end(html);
      return;
    }
    let decoded: string;
    try {
      decoded = decodeURIComponent(path);
    } catch (error) {
      const reason = `malformed path: ${error instanceof Error ? error.message : String(error)}`;
      record(url, 400, reason);
      response.statusCode = 400;
      response.end(reason);
      return;
    }
    const file = resolve(out, `.${decoded}`);
    if (!file.startsWith(out + sep)) {
      const reason = `${path} escapes the built fixture`;
      record(url, 403, reason);
      response.statusCode = 403;
      response.end(reason);
      return;
    }
    readFile(file).then(body => {
      response.setHeader('Content-Type', TYPES[extname(file)] ?? 'application/octet-stream');
      response.setHeader('Content-Length', body.byteLength);
      response.end(body);
    }, error => {
      const code = (error as NodeJS.ErrnoException).code;
      const missing = code === 'ENOENT' || code === 'ENOTDIR' || code === 'EISDIR';
      const status = missing ? 404 : 500;
      const reason = missing ? `${path} is not in the built fixture` : `${path} could not be read (${code ?? String(error)})`;
      record(url, status, reason);
      response.statusCode = status;
      response.end(reason);
    });
  });
  const close = fixtureClose(server, record);
  try {
    await once(server.listen(0, '127.0.0.1'), 'listening');
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    return {
      port: address.port,
      fixture: '/fixture.js',
      failures,
      ready: signal => withHttpProbe(async dispatcher => {
        await fetchFixtureModule(`http://127.0.0.1:${address.port}/fixture.js`, '/fixture.js', failures, dispatcher, signal);
      }),
      close,
    };
  } catch (error) {
    await close();
    throw error;
  }
}

export async function devServer(name: string, route: string, fixture: string, profile: string, plugins: PluginOption[]): Promise<ServedFixture> {
  const failures: ServerFailure[] = [];
  const viteErrors: string[] = [];
  const record = (url: string, status: number | undefined, reason: string) => {
    if (url !== '/favicon.ico') failures.push({ url, status, reason });
  };
  const baseLogger = createLogger('error');
  const customLogger = {
    ...baseLogger,
    error(message: string, options?: Parameters<typeof baseLogger.error>[1]) {
      const cleaned = message.replace(/\x1b\[[0-9;]*m/g, '').replace(/\s+/g, ' ').trim();
      if (cleaned) viteErrors.push(cleaned);
      baseLogger.error(message, options);
    },
  };
  const capturePlugin: PluginOption = {
    name: `${name}-page-capture`,
    enforce: 'pre',
    configureServer: {
      order: 'pre',
      handler(server) {
        server.middlewares.use((request, response, next) => {
          const url = request.url ?? '/';
          if (url === '/favicon.ico') { response.statusCode = 204; response.end(); return; }
          const chunks: Buffer[] = [];
          let captured = 0;
          const push = (chunk: unknown, encoding?: BufferEncoding) => {
            if (response.statusCode < 400 || captured >= 512) return;
            const buffer = captureBodyChunk(chunk, encoding);
            if (!buffer) return;
            const slice = buffer.subarray(0, 512 - captured);
            chunks.push(slice);
            captured += slice.length;
          };
          const write = response.write.bind(response);
          response.write = ((chunk: unknown, encoding?: BufferEncoding | ((error?: Error | null) => void), cb?: (error?: Error | null) => void) => {
            push(chunk, typeof encoding === 'string' ? encoding : undefined);
            return write(chunk as never, encoding as never, cb as never);
          }) as typeof response.write;
          const end = response.end.bind(response);
          response.end = ((chunk?: unknown, encoding?: BufferEncoding | (() => void), cb?: () => void) => {
            push(chunk, typeof encoding === 'string' ? encoding : undefined);
            return end(chunk as never, encoding as never, cb as never);
          }) as typeof response.end;
          response.on('finish', () => {
            if (response.statusCode >= 400) {
              const body = Buffer.concat(chunks).toString('utf8').replace(/\s+/g, ' ').trim();
              const detail = body || response.statusMessage || viteErrors.at(-1) || 'HTTP error';
              record(url, response.statusCode, detail);
            }
          });
          next();
        });
        return () => {
          server.middlewares.use((error: unknown, request: IncomingMessage, _response: ServerResponse, next: (err?: unknown) => void) => {
            const reason = error instanceof Error ? error.message : String(error);
            record(request.url ?? fixture, 500, reason);
            next(error);
          });
        };
      },
    },
  };
  const routePlugin: PluginOption = {
    name: `${name}-page`,
    configureServer(server) {
      server.middlewares.use('/recorded-replies', (request, response) => {
        const url = request.url ?? '/';
        recordedReplies(url.split('?')[0]!, response).catch(error => {
          const reason = error instanceof Error ? error.message : String(error);
          record(`/recorded-replies${url}`, 500, reason);
          response.statusCode = 500;
          response.end(reason);
        });
      });
      server.middlewares.use(route, (_request, response) => {
        const html = `<!doctype html><title>${name}</title>`;
        response.setHeader('Content-Type', 'text/html');
        response.setHeader('Content-Length', Buffer.byteLength(html));
        response.end(html);
      });
    },
  };
  const vite = await createServer({ configFile: false, root: ROOT, customLogger,
    cacheDir: resolve(profile, 'vite'),
    optimizeDeps: { include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', 'react/jsx-dev-runtime', 'react-grid-layout'], noDiscovery: true },
    server: { host: '127.0.0.1', port: 0, watch: null, hmr: false, ws: false },
    plugins: [capturePlugin, ...plugins, routePlugin] });
  const httpServer = vite.httpServer as Server;
  const close = fixtureClose(httpServer, record, () => vite.close());
  let address;
  try {
    await vite.listen();
    address = httpServer.address(); assert.ok(address && typeof address !== 'string');
  } catch (error) {
    await close();
    throw error;
  }
  const origin = `http://127.0.0.1:${address.port}`;
  const ready = (signal?: AbortSignal) => withHttpProbe(async dispatcher => {
    const seen = new Set<string>();
    const queue = [new URL(fixture, origin).href];
    while (queue.length > 0) {
      signal?.throwIfAborted();
      const url = queue.shift()!;
      if (seen.has(url)) continue;
      seen.add(url);
      const fetched = await fetchFixtureModule(url, fixture, failures, dispatcher, signal);
      seen.add(fetched.url);
      for (const imported of staticImportsOf(fetched.code, fetched.url)) {
        if (!seen.has(imported)) queue.push(imported);
      }
    }
  });
  return { port: address.port, fixture, failures, ready, close };
}

export async function withNativePage(name: string, fixture: string, run: (page: NativePage) => Promise<void>, plugins: PluginOption[] = [], built = false, browserArgs: readonly string[] = []): Promise<void> {
  const edge = (process.env.BATTLEPLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLEPLAN_CANVAS_BROWSER to its executable');
  const profile = await mkdtemp(resolve(tmpdir(), `${name}-`));
  const route = `/${name}-page`;
  const readyAbort = new AbortController();
  let ready: Promise<unknown> | undefined;
  let closeFixture: (() => Promise<void>) | undefined;
  let browser: ReturnType<typeof spawn> | undefined, browserPid: number | undefined;
  let browserCdp: NativeCdp | undefined, pageCdp: NativeCdp | undefined;
  try {
    const served = built ? await builtServer(name, route, await builtFixture(name, fixture, plugins)) : await devServer(name, route, fixture, profile, plugins);
    closeFixture = served.close;
    const warming = ready = served.ready(readyAbort.signal).then(() => null, error => error as unknown);
    const address = { port: served.port };
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--no-proxy-server', '--disable-background-networking',
      '--edge-skip-compat-layer-relaunch', '--remote-debugging-port=0', `--user-data-dir=${profile}`, ...browserArgs], { windowsHide: true });
    browser.stdout?.resume(); browser.stderr?.resume();
    const endpoint = await browserStage(browser, `${name} Edge startup`, async signal => {
      for (;;) {
        signal.throwIfAborted();
        try {
          const [port, path] = (await readFile(resolve(profile, 'DevToolsActivePort'), { encoding: 'utf8', signal })).trim().split('\n');
          if (port && path) return `ws://127.0.0.1:${port}/${path.replace(/^\//, '')}`;
        } catch (error) { signal.throwIfAborted(); if (!['ENOENT', 'EBUSY'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error; }
        await pause(50, undefined, { signal });
      }
    });
    const control = browserCdp = new NativeCdp(endpoint);
    const processes = await control.send('SystemInfo.getProcessInfo');
    browserPid = processes.processInfo.find((entry: { type: string }) => entry.type === 'browser')?.id;
    assert.ok(browserPid && Number.isSafeInteger(browserPid));
    const { targetId } = await control.send('Target.createTarget', { url: 'about:blank' });
    const targets = await withHttpProbe(async dispatcher => {
      const response = await fetch(`http://127.0.0.1:${new URL(endpoint).port}/json/list`, { dispatcher, signal: AbortSignal.timeout(30_000) });
      return await response.json() as { id: string; webSocketDebuggerUrl: string }[];
    });
    const target = targets.find(target => target.id === targetId)!.webSocketDebuggerUrl;
    const session = pageCdp = new NativeCdp(target);
    const guard = new AsyncLocalStorage<AbortSignal>();
    const evaluate = async <T = any,>(expression: string): Promise<T> => {
      const result = await session.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, guard.getStore());
      assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
      return result.result.value as T;
    };
    const owned = browser;
    const stage = <T,>(label: string, work: (signal: AbortSignal) => Promise<T>, timeout?: number) =>
      browserStage(owned, label, signal => guard.run(signal, () => work(signal)), timeout);
    const pageUrl = `http://127.0.0.1:${address.port}${route}`;
    const loadPage = (label: string, ignoreCache = false, waitReady = false) =>
      runFixtureLoadStage(stage, label, served, async (signal, networkFailures) => {
        if (waitReady) {
          const onAbort = () => readyAbort.abort(signal.reason);
          signal.addEventListener('abort', onAbort, { once: true });
          try {
            const readyError = await warming;
            if (readyError) throw readyError;
          } finally {
            signal.removeEventListener('abort', onAbort);
          }
        }
        await navigate(session, pageUrl, signal, ignoreCache);
        await loadFixture(session, served, signal, networkFailures);
      }, PAGE_LOAD_MS);
    await loadPage(`${name} page load`, false, true);
    const reload = () => loadPage(`${name} page reload`, true);
    const press = async (key: NativeKey, shift = false) => {
      await session.send('Input.dispatchKeyEvent', { type: 'keyDown', key, code: key, windowsVirtualKeyCode: KEY_CODES[key], modifiers: shift ? 8 : 0,
        ...(key === 'Enter' ? { text: '\r', unmodifiedText: '\r' } : {}) }, guard.getStore());
      await session.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, modifiers: shift ? 8 : 0 }, guard.getStore());
    };
    await run({ evaluate, press, reload, page: target, stage });
  } finally {
    readyAbort.abort();
    try {
      if (browser) await stopBrowser(browser, async (_budget, signal) => { await browserCdp?.send('Browser.close', {}, signal); }, async budget => {
        if (process.platform === 'win32') {
          await stopOwnedProfile(profile, browser!.exitCode !== null || browser!.signalCode !== null ? undefined : browser!.pid, browserPid, ownedEdgeProcesses, budget); return;
        }
        assert.ok(browserPid, 'Only the browser identified by this owned CDP endpoint may be stopped'); process.kill(browserPid, 'SIGKILL');
      }, 5_000, async (_budget, signal) => {
        if (process.platform === 'win32') return (await ownedEdgeProcesses(profile, browser!.exitCode !== null || browser!.signalCode !== null ? undefined : browser!.pid, false, undefined, signal)).length === 0;
        if (!browserPid) return browser!.exitCode !== null || browser!.signalCode !== null;
        try { process.kill(browserPid, 0); return false; } catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') return true; throw error; }
      }, { profile });
    } finally {
      await Promise.all([ready, closeFixture?.(), pageCdp?.close(), browserCdp?.close()]);
    }
    assert.ok(profile.startsWith(resolve(tmpdir()) + sep));
    await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
