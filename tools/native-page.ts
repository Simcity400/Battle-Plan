import assert from 'node:assert/strict';
import { AsyncLocalStorage } from 'node:async_hooks';
import { spawn } from 'node:child_process';
import { createReadStream, existsSync, rmSync } from 'node:fs';
import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, extname, resolve, sep } from 'node:path';
import { setTimeout as pause } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { build, createLogger, createServer, type PluginOption } from 'vite';
import { browserStage, ownedEdgeProcesses, stopBrowser, stopOwnedProfile } from './canvas-page.ts';

function cdp(endpoint: string, method: string, params: Record<string, unknown> = {}, signal?: AbortSignal) {
  signal?.throwIfAborted();
  return new Promise<any>((done, reject) => {
    const socket = new WebSocket(endpoint);
    const cleanup = () => { clearTimeout(timeout); signal?.removeEventListener('abort', abort); socket.close(); };
    const abort = () => { cleanup(); reject(signal!.reason); };
    const timeout = signal ? undefined : setTimeout(() => { cleanup(); reject(new Error(`${method} timed out`)); }, 30_000);
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

function navigate(endpoint: string, url: string, signal: AbortSignal) {
  signal.throwIfAborted();
  return new Promise<void>((done, reject) => {
    const socket = new WebSocket(endpoint);
    const methods = ['Page.enable', 'Page.setLifecycleEventsEnabled', 'Page.navigate'];
    const loaded = new Set<string>();
    let loader: string | undefined;
    const finish = (error?: unknown) => { signal.removeEventListener('abort', abort); socket.close(); if (error) reject(error); else done(); };
    const abort = () => finish(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
    socket.addEventListener('open', () => {
      socket.send(JSON.stringify({ id: 1, method: methods[0] }));
      socket.send(JSON.stringify({ id: 2, method: methods[1], params: { enabled: true } }));
      socket.send(JSON.stringify({ id: 3, method: methods[2], params: { url } }));
    });
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data));
      if (message.error) finish(new Error(`${methods[message.id - 1]}: ${JSON.stringify(message.error)}`));
      else if (message.id === 3 && message.result.errorText) finish(new Error(`Page.navigate to ${url}: ${message.result.errorText}`));
      else if (message.id === 3) {
        loader = message.result.loaderId;
        if (loader !== undefined && loaded.has(loader)) finish();
      } else if (message.method === 'Page.lifecycleEvent' && message.params.name === 'load') {
        loaded.add(message.params.loaderId);
        if (message.params.loaderId === loader) finish();
      }
    });
    socket.addEventListener('error', () => finish(new Error('Page.navigate connection failed')));
    socket.addEventListener('close', () => finish(new Error('Page.navigate connection closed before the page loaded')));
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

async function probeFixture(port: number, fixture: string, failures: ServerFailure[], signal?: AbortSignal): Promise<void> {
  const before = failures.length;
  const target = `http://127.0.0.1:${port}${fixture}`;
  let response: Response;
  try {
    response = await fetch(target, { signal });
  } catch (error) {
    if (signal?.aborted) throw signal.reason;
    const reason = error instanceof Error ? error.message : String(error);
    throw formatFixtureLoadError(fixture, `HTTP probe failed for ${target}: ${reason}`, failures.slice(before), []);
  }
  const contentType = response.headers.get('content-type') ?? '';
  if (!response.ok) {
    const body = (await response.text().catch(() => '')).replace(/\s+/g, ' ').trim().slice(0, 512);
    const serverSeen = failures.slice(before);
    const detail = serverSeen.length > 0 ? '' : `server ${response.status} ${fixture}: ${body || response.statusText || 'HTTP error'}`;
    throw formatFixtureLoadError(fixture, detail, serverSeen, []);
  }
  await response.arrayBuffer();
  if (!/(?:javascript|ecmascript)/i.test(contentType)) {
    throw formatFixtureLoadError(fixture, `server returned non-JavaScript content-type ${JSON.stringify(contentType)} for ${fixture}`, failures.slice(before), []);
  }
}

export function loadFixture(endpoint: string, served: Pick<ServedFixture, 'fixture' | 'failures'>, signal: AbortSignal): Promise<void> {
  signal.throwIfAborted();
  const failuresBefore = served.failures.length;
  return new Promise<void>((done, reject) => {
    const socket = new WebSocket(endpoint);
    const requestUrls = new Map<string, string>();
    const httpFailedUrls = new Set<string>();
    const networkFailures: BrowserNetworkFailure[] = [];
    let settled = false;
    const finish = (error?: unknown) => {
      if (settled) return;
      settled = true;
      signal.removeEventListener('abort', abort);
      socket.close();
      if (error) reject(error); else done();
    };
    const abort = () => finish(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
    socket.addEventListener('open', () => {
      socket.send(JSON.stringify({ id: 1, method: 'Network.enable' }));
    });
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data));
      if (message.id === 1) {
        if (message.error) {
          finish(formatFixtureLoadError(served.fixture, `Network.enable: ${JSON.stringify(message.error)}`, served.failures.slice(failuresBefore), networkFailures));
          return;
        }
        socket.send(JSON.stringify({
          id: 2,
          method: 'Runtime.evaluate',
          params: { expression: `import(${JSON.stringify(served.fixture)}).then(() => true)`, awaitPromise: true, returnByValue: true },
        }));
        return;
      }
      if (message.method === 'Network.requestWillBeSent') {
        requestUrls.set(message.params.requestId, message.params.request.url);
        return;
      }
      if (message.method === 'Network.responseReceived') {
        const { url, status, statusText } = message.params.response;
        if (status >= 400 && !String(url).endsWith('/favicon.ico')) {
          httpFailedUrls.add(url);
          networkFailures.push({ url, status, reason: statusText || 'HTTP error' });
        }
        return;
      }
      if (message.method === 'Network.loadingFailed') {
        const url = requestUrls.get(message.params.requestId) ?? '<unknown>';
        if (!url.endsWith('/favicon.ico') && !(message.params.canceled && httpFailedUrls.has(url))) {
          networkFailures.push({ url, reason: message.params.errorText || 'loading failed' });
        }
        return;
      }
      if (message.id === 2) {
        if (message.error) {
          finish(formatFixtureLoadError(served.fixture, JSON.stringify(message.error), served.failures.slice(failuresBefore), networkFailures));
          return;
        }
        const exception = message.result?.exceptionDetails;
        if (exception) {
          const detail = exception.exception?.description ?? exception.text ?? JSON.stringify(exception);
          finish(formatFixtureLoadError(served.fixture, detail, served.failures.slice(failuresBefore), networkFailures));
          return;
        }
        finish();
      }
    });
    socket.addEventListener('error', () => finish(formatFixtureLoadError(served.fixture, 'CDP connection failed while loading fixture', served.failures.slice(failuresBefore), networkFailures)));
    socket.addEventListener('close', () => finish(formatFixtureLoadError(served.fixture, 'CDP connection closed before the fixture finished loading', served.failures.slice(failuresBefore), networkFailures)));
  });
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
      if (spec.startsWith('/')) {
        imports.push(spec);
      } else if (spec.startsWith('./') || spec.startsWith('../')) {
        const resolved = new URL(spec, `http://127.0.0.1${fromUrl}`);
        imports.push(`${resolved.pathname}${resolved.search}`);
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
  server.keepAliveTimeout = PAGE_LOAD_MS;
  server.headersTimeout = PAGE_LOAD_MS + 1_000;
  server.on('clientError', error => {
    record('<socket>', undefined, `HTTP client error: ${error.message}`);
  });
  await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  return {
    port: address.port,
    fixture: '/fixture.js',
    failures,
    ready: signal => probeFixture(address.port, '/fixture.js', failures, signal),
    close: () => new Promise<void>(done => { server.close(() => done()); server.closeAllConnections(); }),
  };
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
  const vite = await createServer({ configFile: false, root: ROOT, customLogger,
    cacheDir: resolve(profile, 'vite'),
    optimizeDeps: { include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', 'react/jsx-dev-runtime', 'react-grid-layout'], noDiscovery: true },
    server: { host: '127.0.0.1', port: 0, watch: null, hmr: false, ws: false },
    plugins: [...plugins, { name: `${name}-page`, configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const url = request.url ?? '/';
        if (url === '/favicon.ico') { response.statusCode = 204; response.end(); return; }
        const chunks: Buffer[] = [];
        let captured = 0;
        const push = (chunk: unknown, encoding?: BufferEncoding) => {
          if (response.statusCode < 400 || chunk == null || captured >= 512) return;
          const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk), encoding);
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
      return () => {
        server.middlewares.use((error: unknown, request: IncomingMessage, _response: ServerResponse, next: (err?: unknown) => void) => {
          const reason = error instanceof Error ? error.message : String(error);
          record(request.url ?? fixture, 500, reason);
          next(error);
        });
      };
    } }] });
  await vite.listen();
  const httpServer = vite.httpServer!;
  if ('keepAliveTimeout' in httpServer) {
    httpServer.keepAliveTimeout = PAGE_LOAD_MS;
    httpServer.headersTimeout = PAGE_LOAD_MS + 1_000;
  }
  httpServer.on('clientError', error => {
    record('<socket>', undefined, `HTTP client error: ${error.message}`);
  });
  const address = httpServer.address(); assert.ok(address && typeof address !== 'string');
  const ready = async (signal?: AbortSignal) => {
    const seen = new Set<string>();
    const queue = [fixture];
    while (queue.length > 0) {
      signal?.throwIfAborted();
      const url = queue.shift()!;
      if (seen.has(url)) continue;
      seen.add(url);
      let transformed: Awaited<ReturnType<typeof vite.transformRequest>>;
      try {
        transformed = await vite.transformRequest(url);
      } catch (error) {
        if (signal?.aborted) throw signal.reason;
        const reason = error instanceof Error ? error.message : String(error);
        record(url, 500, reason);
        throw formatFixtureLoadError(fixture, '', [{ url, status: 500, reason }], []);
      }
      if (!transformed) {
        if (url === fixture) {
          const reason = `${fixture} could not be resolved by Vite`;
          record(fixture, 404, reason);
          throw formatFixtureLoadError(fixture, '', [{ url: fixture, status: 404, reason }], []);
        }
        continue;
      }
      for (const imported of staticImportsOf(transformed.code, url)) {
        if (!seen.has(imported)) queue.push(imported);
      }
    }
    await probeFixture(address.port, fixture, failures, signal);
  };
  return { port: address.port, fixture, failures, ready, close: () => vite.close() };
}

export async function withNativePage(name: string, fixture: string, run: (page: NativePage) => Promise<void>, plugins: PluginOption[] = [], built = false, browserArgs: readonly string[] = []): Promise<void> {
  const edge = (process.env.BATTLEPLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLEPLAN_CANVAS_BROWSER to its executable');
  const profile = await mkdtemp(resolve(tmpdir(), `${name}-`));
  const route = `/${name}-page`;
  const served = built ? await builtServer(name, route, await builtFixture(name, fixture, plugins)) : await devServer(name, route, fixture, profile, plugins);
  const readyAbort = new AbortController();
  const ready = served.ready(readyAbort.signal).then(() => null, error => error as unknown);
  let browser: ReturnType<typeof spawn> | undefined, endpoint = '', browserPid: number | undefined;
  try {
    const address = { port: served.port };
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--no-proxy-server', '--disable-background-networking',
      '--edge-skip-compat-layer-relaunch', '--remote-debugging-port=0', `--user-data-dir=${profile}`, ...browserArgs], { windowsHide: true });
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
    const guard = new AsyncLocalStorage<AbortSignal>();
    const evaluate = async <T = any,>(expression: string): Promise<T> => {
      const result = await cdp(target, 'Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, guard.getStore());
      assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
      return result.result.value as T;
    };
    const owned = browser;
    const stage = <T,>(label: string, work: (signal: AbortSignal) => Promise<T>, timeout?: number) =>
      browserStage(owned, label, signal => guard.run(signal, () => work(signal)), timeout);
    await stage(`${name} page load`, async signal => {
      const onAbort = () => readyAbort.abort(signal.reason);
      signal.addEventListener('abort', onAbort, { once: true });
      try {
        const readyError = await ready;
        if (readyError) throw readyError;
      } finally {
        signal.removeEventListener('abort', onAbort);
      }
      await navigate(target, `http://127.0.0.1:${address.port}${route}`, signal);
      await loadFixture(target, served, signal);
    }, PAGE_LOAD_MS);
    const press = async (key: NativeKey, shift = false) => {
      await cdp(target, 'Input.dispatchKeyEvent', { type: 'keyDown', key, code: key, windowsVirtualKeyCode: KEY_CODES[key], modifiers: shift ? 8 : 0 }, guard.getStore());
      await cdp(target, 'Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, modifiers: shift ? 8 : 0 }, guard.getStore());
    };
    await run({ evaluate, press, page: target, stage });
  } finally {
    readyAbort.abort();
    await ready;
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
