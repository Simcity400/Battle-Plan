import { asFailure, failureWire, queryFailure } from './src/dashboard/queryError.ts';
import { execFile, spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, open, readdir, readFile, realpath, rm, stat } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createInterface } from 'node:readline';
import type { Plugin } from 'vite';
import { listReplayFolder, savedReplayFolder, saveReplayFolder } from './replayLibrary.ts';
import { BROWSER_BRIDGE_BUSY, MAX_MESSAGE_BYTES } from './src/generated/dashboard.ts';
import type { PartialRecording } from './src/generated/dashboard.ts';
import type { ReplayLibrary } from './src/dashboard/client.ts';
import { artworkName, BROWSER_ARTWORK_BASE } from './src/dashboard/payload.ts';

const runFile = promisify(execFile);

export async function recordingRequestBody(request: AsyncIterable<Buffer | string>): Promise<unknown> {
  const chunks: Buffer[] = [];
  let bytes = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    bytes += buffer.length;
    if (bytes > MAX_MESSAGE_BYTES) throw new Error('Recording request too large');
    chunks.push(buffer);
  }
  return JSON.parse(Buffer.concat(chunks, bytes).toString('utf8'));
}

export function bridgeCommand(command: Record<string, unknown>): string {
  const line = `${JSON.stringify(command)}\n`;
  if (Buffer.byteLength(line) > MAX_MESSAGE_BYTES) throw new Error('Bridge command exceeds its byte budget');
  return line;
}

export function mapSettingsCommand(input: Record<string, any>): Record<string, unknown> {
  return {
    type: 'mapSettings', sourceId: input.sourceId, revision: input.revision, settings: input.settings ?? null,
    flip: input.flip ?? null, target: input.target ?? { scope: 'campaign' },
  };
}

export function preparedQueryCommand(input: Record<string, any>): Record<string, unknown> {
  if (!(input.refresh == null || typeof input.refresh === 'boolean')) throw new Error('Invalid prepared query request');
  return { type: 'preparedQuery', request: input.request, refresh: input.refresh ?? false };
}

export function pageRecordings(release: (sourceId: string) => void, remove: (target: string) => void) {
  const imports = new Map<string, { page: string; opening: number; sources: Set<string> }>();
  const sources = new Map<string, { page: string; target: string | null }>();
  const gone = new Set<string>();
  const settle = (target: string) => {
    const entry = imports.get(target);
    if (!entry || !gone.has(entry.page) || entry.opening || entry.sources.size) return;
    imports.delete(target);
    remove(target);
  };
  const drop = (sourceId: string) => {
    const source = sources.get(sourceId);
    if (!source) return;
    sources.delete(sourceId);
    if (source.target !== null) { imports.get(source.target)?.sources.delete(sourceId); settle(source.target); }
  };
  return {
    isImport: (target: string) => imports.has(target),
    imported: (target: string, page: string) => { imports.set(target, { page, opening: 0, sources: new Set() }); settle(target); },
    open: (path: string, page: string) => {
      const entry = imports.get(path);
      if (entry) entry.opening++;
      let ended = false;
      return (sourceId?: string) => {
        if (ended) return;
        ended = true;
        if (sourceId !== undefined && gone.has(page)) release(sourceId);
        else if (sourceId !== undefined) { sources.set(sourceId, { page, target: entry ? path : null }); entry?.sources.add(sourceId); }
        if (entry) { entry.opening--; settle(path); }
      };
    },
    released: (sourceId: string) => drop(sourceId),
    left: (page: string) => {
      gone.add(page);
      for (const [sourceId, source] of [...sources]) if (source.page === page) { release(sourceId); drop(sourceId); }
      for (const [target, entry] of [...imports]) if (entry.page === page) settle(target);
    },
  };
}

function pageId(value: unknown): string {
  if (typeof value !== 'string' || !value || value.length > 64) throw new Error('Invalid recording request');
  return value;
}

export function browserBridge(root: string, recordingsRoot: string, gameDir: string, userDir: string): Plugin {
  const libraryRoot = savedReplayFolder(recordingsRoot).then(folder => { recordingsRoot = folder; });
  let contentConfig: Promise<void> | null = null;
  const loadContentConfig = () => contentConfig ??= (async () => {
    if (!process.env.APPDATA) return;
    const bytes = await readFile(path.join(process.env.APPDATA, 'battle-plan', 'content.json'), 'utf8')
      .catch(error => { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null; throw error; });
    if (bytes === null) return;
    const saved: { game_dir?: string; user_dir?: string } = JSON.parse(bytes);
    gameDir = saved.game_dir || gameDir;
    userDir = saved.user_dir || userDir;
  })().catch(error => { contentConfig = null; throw error; });
  const importsRoot = path.join(tmpdir(), 'battle-plan-imports');
  const importsOwned = path.join(importsRoot, `${process.pid}-${randomUUID()}`);
  const running = (pid: number) => {
    try { process.kill(pid, 0); return true; } catch (error) { return (error as NodeJS.ErrnoException).code === 'EPERM'; }
  };
  const sweepImports = async () => {
    const entries = await readdir(importsRoot).catch(() => [] as string[]);
    await Promise.all(entries.map(entry => {
      const owner = /^(\d+)-/.exec(entry);
      return owner && Number(owner[1]) !== process.pid && !running(Number(owner[1])) ? rm(path.join(importsRoot, entry), { recursive: true, force: true }).catch(() => {}) : undefined;
    }));
  };
  let child: ChildProcessWithoutNullStreams | null = null;
  let picker: AbortController | null = null;
  const pending = new Map<string, { resolve: (value: unknown) => void; reject: (error: Error) => void; parts: string[]; bytes: number; sequence: number; timer: ReturnType<typeof setTimeout> }>();
  let next = 0;
  const exits = new Set<Promise<void>>();
  let stopped: Promise<void> | null = null;
  const fail = (message: string) => {
    for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(new Error(message)); }
    pending.clear();
    const owned = child; child = null;
    if (owned) { owned.stdin.end(); if (owned.exitCode === null && owned.signalCode === null) owned.kill(); }
  };
  const start = () => {
    if (child) return child;
    const args = [...(gameDir ? ['--game-dir', gameDir] : []), ...(userDir ? ['--user-dir', userDir] : []),
      '--artwork-url', BROWSER_ARTWORK_BASE];
    const process = spawn(path.join(root, '.cargo-target/debug/dashboard-bridge.exe'), args, { cwd: root, windowsHide: true });
    const closed = new Promise<void>(resolve => process.once('close', () => resolve()));
    exits.add(closed); void closed.then(() => exits.delete(closed));
    child = process;
    process.on('error', () => { if (child === process) fail('Build the browser bridge with cargo build -p dashboard --bin dashboard-bridge.'); });
    process.on('exit', () => { if (child === process) fail('The recording bridge stopped. Reopen the recording to reconnect.'); });
    process.stdin.on('error', () => { if (child === process) fail('The recording bridge connection closed. Reopen the recording to reconnect.'); });
    process.stderr.on('data', (data: Buffer) => console.error(`[recording bridge] ${data.toString().slice(0, 2048)}`));
    createInterface({ input: process.stdout }).on('line', (line) => {
      try {
        const event = JSON.parse(line);
        const entry = pending.get(event.id);
        if (!entry) return;
        if (event.type === 'error') { clearTimeout(entry.timer); pending.delete(event.id); entry.reject(queryFailure(event.error)); return; }
        if (event.type !== 'chunk' || event.sequence !== entry.sequence++ || typeof event.fragment !== 'string') throw new Error('Invalid bridge response');
        entry.bytes += Buffer.byteLength(event.fragment);
        if (entry.bytes > 32 * 1024 * 1024 || [...pending.values()].reduce((bytes, item) => bytes + item.bytes, 0) > 64 * 1024 * 1024) throw new Error('Bridge response budget exceeded');
        entry.parts.push(event.fragment);
        if (event.last) {
          if (entry.bytes !== event.totalBytes) throw new Error('Incomplete bridge response');
          const value = JSON.parse(entry.parts.join(''));
          clearTimeout(entry.timer); pending.delete(event.id); entry.resolve(value);
        }
      } catch (error) { fail(error instanceof Error ? error.message : String(error)); }
    });
    return process;
  };
  const send = async (command: Record<string, unknown>, signal: AbortSignal) => { await loadContentConfig(); return sendLoaded(command, signal); };
  const sendLoaded = (command: Record<string, unknown>, signal: AbortSignal) => new Promise<unknown>((resolve, reject) => {
    if (signal.aborted) { reject(new Error('Request cancelled')); return; }
    if (pending.size >= 64) { reject(new Error(BROWSER_BRIDGE_BUSY)); return; }
    const id = typeof command.id === 'string' ? command.id : `browser-${++next}`;
    if (pending.has(id)) { reject(new Error('Duplicate recording request')); return; }
    let line: string;
    try { line = bridgeCommand({ ...command, id }); }
    catch (error) { reject(error); return; }
    const process = start();
    const cancel = () => {
      const entry = pending.get(id);
      if (!entry) return;
      clearTimeout(entry.timer); pending.delete(id); signal.removeEventListener('abort', cancel);
      process.stdin.write(bridgeCommand({ type: 'cancel', id }));
      reject(new Error('Recording request cancelled or timed out'));
    };
    const finish = (done: () => void) => { signal.removeEventListener('abort', cancel); done(); };
    pending.set(id, { resolve: value => finish(() => resolve(value)), reject: error => finish(() => reject(error)), parts: [], bytes: 0, sequence: 0, timer: setTimeout(cancel, 180_000) });
    signal.addEventListener('abort', cancel, { once: true });
    process.stdin.write(line);
  });
  const bridgeLetGo = async () => { if (child) await send({ type: 'recordingSummaries', paths: [] }, new AbortController().signal).catch(() => {}); };
  const pages = pageRecordings(sourceId => { child?.stdin.write(bridgeCommand({ type: 'release', sourceId })); }, target => {
    void bridgeLetGo().then(() => rm(path.dirname(target), { recursive: true, force: true, maxRetries: 5, retryDelay: 200 })).catch(() => {});
  });
  const requireBridge = async (source: string, message: string) => {
    const binary = await stat(path.join(root, '.cargo-target/debug/dashboard-bridge.exe'));
    if (binary.mtimeMs < (await stat(path.join(root, source))).mtimeMs) throw new Error(message);
  };
  const withinRoot = async (relative: string) => {
    if (pages.isImport(relative)) return { target: await realpath(relative) };
    if (!recordingsRoot) throw new Error('Choose a replay folder in Settings.');
    const base = await realpath(recordingsRoot);
    const target = await realpath(path.resolve(base, relative));
    const childPath = path.relative(base, target);
    if (childPath.startsWith('..') || path.isAbsolute(childPath)) throw new Error('Choose a recording inside the configured recordings folder.');
    return { target, relative: childPath.replaceAll('\\', '/') };
  };
  const stop = () => {
    picker?.abort(); fail('Development server stopped');
    return stopped ??= Promise.all(exits).then(() => rm(importsOwned, { recursive: true, force: true })).catch(() => {});
  };
  const recordingSummaries = async (library: ReplayLibrary, signal: AbortSignal) => {
    for (let first = 0; first < library.files.length;) {
      const paths: string[] = [];
      let bytes = 0;
      while (first + paths.length < library.files.length) {
        const target = library.files[first + paths.length].path;
        const size = Buffer.byteLength(JSON.stringify(target)) + 1;
        if (paths.length && (bytes + size > 56 * 1024 || paths.length >= 128)) break;
        paths.push(target); bytes += size;
      }
      const partials = await send({ type: 'recordingSummaries', paths }, signal) as (PartialRecording | null)[];
      if (!Array.isArray(partials) || partials.length !== paths.length) throw new Error('Incomplete recording summaries');
      for (let index = 0; index < paths.length; index++) library.files[first + index].partial = partials[index];
      first += paths.length;
    }
    return library;
  };
  return {
    name: 'battle-plan-browser-recordings', apply: 'serve',
    async closeBundle() { await stop(); },
    configureServer(server) {
      void sweepImports();
      server.httpServer?.once('close', () => { void stop(); });
      server.middlewares.use(BROWSER_ARTWORK_BASE.slice(0, -1), async (request, response) => {
        const name = new URL(request.url ?? '/', 'http://localhost').pathname.slice(1);
        const controller = new AbortController();
        response.on('close', () => { if (!response.writableEnded) controller.abort(); });
        const png = child && (request.method === 'GET' || request.method === 'HEAD') && artworkName(name)
          ? await send({ type: 'artwork', name }, controller.signal).then(value => typeof value === 'string' ? Buffer.from(value, 'base64') : null, () => null)
          : null;
        if (!png) { response.statusCode = 404; response.end(); return; }
        response.setHeader('Content-Type', 'image/png');
        response.setHeader('Cache-Control', 'public, max-age=31536000, immutable');
        response.end(request.method === 'HEAD' ? undefined : png);
      });
      server.middlewares.use('/api/recordings', async (request, response) => {
        response.setHeader('Content-Type', 'application/json');
        response.setHeader('Cache-Control', 'no-store');
        const controller = new AbortController();
        response.on('close', () => { if (!response.writableEnded) controller.abort(); });
        try {
          await libraryRoot;
          if (request.headers.origin && request.headers.origin !== `http://${request.headers.host}`) throw new Error('Cross-origin recording access is disabled');
          if (new URL(request.url ?? '/', 'http://localhost').pathname === '/import' && request.method === 'POST') {
            if (request.headers['content-type'] !== 'application/octet-stream') throw new Error('Expected a recording file');
            const name = path.basename(decodeURIComponent(String(request.headers['x-recording-name'] ?? ''))).replace(/[<>:"/\\|?*\x00-\x1f]/g, '_');
            if (!(/\.(?:battleplan|hoi4replay)$/i.test(name)) || name.length > 180) throw new Error('Choose a .battleplan recording');
            const page = pageId(request.headers['x-recording-page']);
            const target = path.join(importsOwned, randomUUID(), name);
            await mkdir(path.dirname(target), { recursive: true });
            const file = await open(target, 'wx');
            let bytes = 0;
            try {
              for await (const chunk of request) {
                bytes += chunk.length;
                if (bytes > 2 * 1024 * 1024 * 1024) throw new Error('Open recordings larger than 2 GB in the desktop app.');
                await file.writeFile(chunk);
              }
              if (!bytes) throw new Error('The recording is empty');
              await file.close();
              const canonical = await realpath(target);
              pages.imported(canonical, page);
              response.end(JSON.stringify({ path: canonical }));
            } catch (error) { await file.close(); await rm(path.dirname(target), { recursive: true, force: true }).catch(() => {}); throw error; }
            return;
          }
          if (request.method !== 'POST' || !request.headers['content-type']?.startsWith('application/json')) throw new Error('Expected a JSON recording request');
          const input = await recordingRequestBody(request) as Record<string, any>;
          if (input.type === 'release' || input.type === 'pageGone') {
            if (input.type === 'pageGone') pages.left(pageId(input.page));
            else {
              child?.stdin.write(bridgeCommand({ type: 'release', sourceId: input.sourceId }));
              if (typeof input.sourceId === 'string') pages.released(input.sourceId);
            }
            response.end('{}'); return;
          }
          if (pending.size >= 64) throw new Error(BROWSER_BRIDGE_BUSY);
          if (input.type === 'open') {
            const ended = pages.open(String(input.path), pageId(input.page));
            let sourceId: string | undefined;
            try {
              const location = await withinRoot(String(input.path));
              if (!(/\.(?:battleplan|hoi4replay)$/i.test(location.target)) || !(await stat(location.target)).isFile()) throw new Error('Choose a .battleplan recording');
              const source = await send({ type: 'open', path: location.target, revision: '' }, controller.signal);
              const id = (source as { id?: unknown } | null)?.id;
              if (typeof id === 'string') sourceId = id;
              response.end(JSON.stringify(source));
            } finally { ended(sourceId); }
            return;
          }
          if (input.type === 'library') {
            response.end(JSON.stringify(await recordingSummaries(await listReplayFolder(recordingsRoot, controller.signal), controller.signal))); return;
          }
          if (input.type === 'contentFolders') {
            await loadContentConfig();
            const game = gameDir;
            const user = userDir;
            const contentReady = !!game && !!user && await stat(path.join(game, 'hoi4.exe')).then(s => s.isFile(), () => false)
              && await stat(user).then(s => s.isDirectory(), () => false);
            response.end(JSON.stringify({ game, user, contentReady })); return;
          }
          if (input.type === 'chooseFolder') {
            if (!['game', 'user', 'replay'].includes(input.kind)) throw new Error('Unknown folder type');
            if (typeof input.initial !== 'string' || input.initial.length > 32000 || input.initial.includes('\0')) throw new Error('Invalid starting folder');
            if (picker) throw new Error('A folder picker is already open. Finish choosing that folder first.');
            picker = controller;
            try {
              const initial = input.initial || (input.kind === 'replay' ? recordingsRoot : '');
              const { stdout } = await runFile(path.join(root, '.cargo-target/debug/dashboard-bridge.exe'), ['--choose-folder', input.kind, initial], {
                windowsHide: true, encoding: 'utf8', maxBuffer: 128 * 1024, timeout: 300_000, signal: controller.signal,
              });
              response.end(JSON.stringify(JSON.parse(stdout)));
            } finally { picker = null; }
            return;
          }
          if (input.type === 'setContentFolders') {
            if ([input.game, input.user].some(value => typeof value !== 'string' || !value.trim() || value.length > 32000 || value.includes('\0'))) throw new Error('Choose the game and user folders.');
            const value = await send({ type: 'setContentFolders', game: input.game, user: input.user }, controller.signal);
            gameDir = input.game; userDir = input.user;
            response.end(JSON.stringify(value)); return;
          }
          if (input.type === 'setReplayFolder') {
            if (typeof input.folder !== 'string' || input.folder.length > 32000) throw new Error('Choose a replay folder.');
            const library = await saveReplayFolder(input.folder);
            recordingsRoot = library.folder!;
            response.end(JSON.stringify(await recordingSummaries(library, controller.signal))); return;
          }
          let command: Record<string, unknown>;
          if (input.type === 'mapSettings') {
            await requireBridge('crates/dashboard-core/src/map_settings.rs', 'Map settings are unavailable in this server version. The local data bridge needs updating.');
            command = mapSettingsCommand(input);
          } else if (input.type === 'preparationPreferences') {
            if (typeof input.id !== 'string' || !(input.enabled == null || typeof input.enabled === 'boolean')) throw new Error('Invalid preparation preference request');
            command = { type: 'preparationPreferences', id: input.id, enabled: input.enabled ?? null };
          } else if (input.type === 'preparation') {
            if (typeof input.id !== 'string' || typeof input.sourceId !== 'string' || typeof input.revision !== 'string' || !['status', 'start', 'cancel', 'clear'].includes(input.action)) throw new Error('Invalid preparation request');
            command = { type: 'preparation', id: input.id, sourceId: input.sourceId, revision: input.revision, action: input.action };
          } else if (input.type === 'preparedQuery') {
            command = preparedQueryCommand(input);
          } else if (input.type === 'product') {
            command = { type: 'product', request: input.request };
          } else throw new Error('Unknown recording operation');
          response.end(JSON.stringify(await send(command, controller.signal)));
        } catch (error) {
          response.statusCode = 400;
          response.end(JSON.stringify({ error: failureWire(asFailure(error)) }));
        }
      });
    },
  };
}
