import { readdir, readFile, realpath, mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import type { ReplayFile, ReplayLibrary } from './src/dashboard/client.ts';

const settingsPath = path.join(process.env.APPDATA || path.join(homedir(), '.config'), 'battle-plan', 'replay-folder.json');

export async function savedReplayFolder(fallback: string): Promise<string> {
  try { const settings = JSON.parse(await readFile(settingsPath, 'utf8')); return typeof settings.folder === 'string' ? settings.folder : fallback; }
  catch { return fallback; }
}

export async function listReplayFolder(folder: string, signal?: AbortSignal): Promise<ReplayLibrary> {
  if (!folder) return { folder: null, files: [] };
  const root = await realpath(folder);
  const files: ReplayFile[] = [];
  const pending = [root];
  let visited = 0;
  while (pending.length) {
    signal?.throwIfAborted();
    const directory = pending.pop()!;
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      if (++visited > 50000) throw new Error('This folder is too large. Choose a folder containing only your replays.');
      if (entry.isSymbolicLink() || entry.name.startsWith('.')) continue;
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) pending.push(full);
      else if (entry.isFile() && (/\.(?:battleplan|hoi4replay)$/i.test(entry.name))) {
        files.push({ path: full, name: entry.name.slice(0, -'.battleplan'.length), folder: path.relative(root, directory).replaceAll('\\', '/') });
      }
    }
  }
  files.sort((a, b) => `${a.folder}/${a.name}`.localeCompare(`${b.folder}/${b.name}`));
  return { folder: root, files };
}

export async function saveReplayFolder(folder: string): Promise<ReplayLibrary> {
  const library = await listReplayFolder(folder);
  if (!library.folder) throw new Error('Choose a replay folder.');
  await mkdir(path.dirname(settingsPath), { recursive: true });
  await writeFile(settingsPath, JSON.stringify({ folder: library.folder }), 'utf8');
  return library;
}
