import { mkdir, copyFile, symlink, utimes, rm, cp, lstat } from 'node:fs/promises';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';
import { browserBridge } from '../browserBridge.ts';
import { repository } from './settings.mjs';
import { copyPreparedSeed } from './nativeSeed.mjs';
import { writeJSON } from './cache.mjs';

export async function removeAdapter(root) {
  // Unlink the input junction before recursive scratch removal.
  await rm(path.join(root, '.cargo-target/debug/shader-tools'), { force: true, recursive: true });
  await rm(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
}

export async function startUI(ui, config, output) {
  if (/(?:^|[\\/])debug(?:[\\/]|$)/i.test(config.bridge)) throw Error(`Refusing debug bridge: ${config.bridge}`);
  const root = path.join(output, 'bridge'), binary = path.join(root, '.cargo-target/debug/dashboard-bridge.exe');
  const prior = { APPDATA: process.env.APPDATA, LOCALAPPDATA: process.env.LOCALAPPDATA };
  let server;
  const close = async () => {
    try { await server?.close(); }
    finally {
      for (const [key, value] of Object.entries(prior)) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
      await removeAdapter(root);
    }
  };
  try {
    await mkdir(path.dirname(binary), { recursive: true }); await copyFile(config.bridge, binary);
    await symlink(path.resolve(config.shaderTools), path.join(path.dirname(binary), 'shader-tools'), process.platform === 'win32' ? 'junction' : 'dir');
    for (const name of ['crates/dashboard-core/src/map_settings.rs', 'crates/dashboard/src/reply_cache.rs']) {
      const marker = path.join(root, name);
      await mkdir(path.dirname(marker), { recursive: true }); await copyFile(path.join(ui.root, name), marker);
      // An uncommitted edit keeps its own time, so a bridge built before it is refused.
      if (execFileSync('git', ['status', '--porcelain', '--', name], { cwd: ui.root, encoding: 'utf8' }).trim()) continue;
      const changed = new Date(Number(execFileSync('git', ['log', '-1', '--format=%ct', '--', name], { cwd: ui.root, encoding: 'utf8' })) * 1000);
      await utimes(marker, changed, changed);
    }
    // Native verification checks the cached file identities again. Copy its
    // existing index so each adapter keeps private writes without rehashing an
    // unchanged installation before every comparison role.
    const seed = config.nativeSeed ?? prior.LOCALAPPDATA;
    if (seed) {
      const relative = 'HOI4Replay/viewer/content-hashes-v2.bin', destination = path.join(root, 'localappdata', relative);
      await mkdir(path.dirname(destination), { recursive: true });
      await copyFile(path.join(seed, relative), destination).catch(error => { if (error.code !== 'ENOENT') throw error; });
      await cp(path.join(seed, 'HOI4Replay/viewer/history'), path.join(root, 'localappdata/HOI4Replay/viewer/history'), {
        recursive: true, filter: async input => {
          if ((await lstat(input)).isSymbolicLink()) throw Error(`Linked native history input refused: ${input}`);
          return true;
        },
      }).catch(error => { if (error.code !== 'ENOENT') throw error; });
    }
    const preparedSeed = await copyPreparedSeed(config.preparedSeed, path.join(root, 'localappdata/HOI4Replay/prepared'));
    const nativeInputs = { historySeed: seed ?? null, preparedSeed };
    await writeJSON(path.join(output, 'native-inputs.json'), nativeInputs);
    process.env.APPDATA = path.join(root, 'appdata'); process.env.LOCALAPPDATA = path.join(root, 'localappdata');
    server = await createServer({ configFile: false, root: path.join(ui.root, 'ui'), mode: 'parity', cacheDir: path.join(root, 'vite'),
      plugins: [react(), browserBridge(root, path.dirname(config.recording), config.gameDir, config.userDir)], clearScreen: false, logLevel: 'warn',
      server: { host: '127.0.0.1', port: 0, strictPort: true, fs: { allow: [ui.root, repository] } } });
    await server.listen();
    return { url: `http://127.0.0.1:${server.httpServer.address().port}`, nativeInputs, close };
  } catch (error) { await close(); throw error; }
}
