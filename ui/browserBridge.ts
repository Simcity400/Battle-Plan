import { execFile, spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, open, readFile, realpath, stat, unlink } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createInterface } from 'node:readline';
import type { Plugin } from 'vite';
import { listReplayFolder, savedReplayFolder, saveReplayFolder } from './replayLibrary.ts';
import { BROWSER_BRIDGE_BUSY, MAX_MESSAGE_BYTES } from './src/generated/dashboard.ts';
import type { PartialRecording } from './src/generated/dashboard.ts';
import type { ReplayLibrary } from './src/dashboard/client.ts';

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

export function browserBridge(root: string, recordingsRoot: string, gameDir: string, userDir: string): Plugin {
  const libraryRoot = savedReplayFolder(recordingsRoot).then(folder => { recordingsRoot = folder; });
  let contentConfig: Promise<void> | null = null;
  const loadContentConfig = () => contentConfig ??= readFile(path.join(root, '.cargo-target/debug/viewer.json'), 'utf8').then(bytes => {
    const saved: { game_dir?: string; user_dir?: string } = JSON.parse(bytes);
    gameDir = saved.game_dir || gameDir;
    userDir = saved.user_dir || userDir;
  }).catch(error => { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; });
  const importedPaths = new Set<string>();
  let child: ChildProcessWithoutNullStreams | null = null;
  let picker: AbortController | null = null;
  const pending = new Map<string, { resolve: (value: unknown) => void; reject: (error: Error) => void; parts: string[]; bytes: number; sequence: number; timer: ReturnType<typeof setTimeout> }>();
  let next = 0;
  const fail = (message: string) => {
    for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(new Error(message)); }
    pending.clear(); child?.kill(); child = null;
  };
  const start = () => {
    if (child) return child;
    const args = [...(gameDir ? ['--game-dir', gameDir] : []), ...(userDir ? ['--user-dir', userDir] : [])];
    const process = spawn(path.join(root, '.cargo-target/debug/dashboard-bridge.exe'), args, { cwd: root, windowsHide: true });
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
        if (event.type === 'error') { clearTimeout(entry.timer); pending.delete(event.id); entry.reject(new Error(event.message)); return; }
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
  const send = (command: Record<string, unknown>, signal: AbortSignal) => new Promise<unknown>((resolve, reject) => {
    if (signal.aborted) { reject(new Error('Request cancelled')); return; }
    // Match the desktop service and native loader admission budget. A saved
    // dashboard can have more concurrent consumers than the old browser limit.
    if (pending.size >= 64) { reject(new Error(BROWSER_BRIDGE_BUSY)); return; }
    const id = `browser-${++next}`;
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
  const withinRoot = async (relative: string) => {
    if (importedPaths.has(relative)) return { target: await realpath(relative) };
    if (!recordingsRoot) throw new Error('Choose a replay folder in Settings.');
    const base = await realpath(recordingsRoot);
    const target = await realpath(path.resolve(base, relative));
    const childPath = path.relative(base, target);
    if (childPath.startsWith('..') || path.isAbsolute(childPath)) throw new Error('Choose a recording inside the configured recordings folder.');
    return { target, relative: childPath.replaceAll('\\', '/') };
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
    name: 'hoi4-browser-recordings', apply: 'serve',
    configureServer(server) {
      server.httpServer?.once('close', () => { picker?.abort(); fail('Development server stopped'); });
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
            const target = path.join(tmpdir(), 'battle-plan-imports', randomUUID(), name);
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
              importedPaths.add(canonical);
              response.end(JSON.stringify({ path: canonical }));
            } catch (error) { await file.close(); await unlink(target).catch(() => {}); throw error; }
            return;
          }
          if (request.method !== 'POST' || !request.headers['content-type']?.startsWith('application/json')) throw new Error('Expected a JSON recording request');
          if (pending.size >= 64) throw new Error(BROWSER_BRIDGE_BUSY);
          const input = await recordingRequestBody(request) as Record<string, any>;
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
            await loadContentConfig();
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
          await loadContentConfig();
          if (input.type === 'open') {
            const location = await withinRoot(String(input.path));
            if (!(/\.(?:battleplan|hoi4replay)$/i.test(location.target)) || !(await stat(location.target)).isFile()) throw new Error('Choose a .battleplan recording');
            command = { type: 'open', path: location.target, revision: '' };
          } else if (input.type === 'mapSettings') {
            const binary = await stat(path.join(root, '.cargo-target/debug/dashboard-bridge.exe'));
            const source = await stat(path.join(root, 'crates/dashboard/src/map_settings.rs'));
            if (binary.mtimeMs < source.mtimeMs) throw new Error('Map settings are unavailable in this server version. The local data bridge needs updating.');
            command = { type: 'mapSettings', sourceId: input.sourceId, revision: input.revision, settings: input.settings ?? null, target: input.target ?? { scope: 'campaign' } };
          } else if (input.type === 'product') {
            command = { type: 'product', request: input.request };
          } else if (input.type === 'release') {
            child?.stdin.write(bridgeCommand({ type: 'release', sourceId: input.sourceId }));
            response.end('{}'); return;
          } else throw new Error('Unknown recording operation');
          response.end(JSON.stringify(await send(command, controller.signal)));
        } catch (error) {
          response.statusCode = 400;
          response.end(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
        }
      });
    },
  };
}
