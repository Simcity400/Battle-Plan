import test from 'node:test';
import assert from 'node:assert/strict';
import { cp, mkdtemp, mkdir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

const root = new URL('../../../', import.meta.url);
const entry = await readFile(new URL('ui/src/main.tsx', root), 'utf8');
const fonts = [...entry.matchAll(/import '@fontsource-variable\/([^']+)';/g)].map(match => match[1]);

test('every bundled font retains its exact upstream attribution and complete licence', async () => {
  assert.equal(fonts.length, 5);
  for (const font of fonts) {
    const upstream = await readFile(new URL(`LICENSE`, import.meta.resolve(`@fontsource-variable/${font}`)));
    const shipped = await readFile(new URL(`LICENSES/Fontsource-${font}-LICENSE.txt`, root));
    // Git may check text notices out with CRLF on Windows; their complete text stays exact.
    assert.equal(shipped.toString().replaceAll('\r\n', '\n'), upstream.toString().replaceAll('\r\n', '\n'), `${font}: retain the upstream notice unchanged`);
    assert.match(shipped.toString(), /SIL OPEN FONT LICENSE Version 1\.1/);
  }
});

test('desktop build stages all bundled font notices beside the release executables', async () => {
  const staging = await mkdtemp(path.join(tmpdir(), 'replay-font-notices-'));
  try {
    await cp(new URL('LICENSE', root), path.join(staging, 'LICENSE'));
    await cp(new URL('LICENSES', root), path.join(staging, 'LICENSES'), { recursive: true });
    // A previous packaging script may also require the repository's notice index.
    await cp(new URL('THIRD_PARTY_NOTICES.md', root), path.join(staging, 'THIRD_PARTY_NOTICES.md'))
      .catch((error: NodeJS.ErrnoException) => { if (error.code !== 'ENOENT') throw error; });
    await mkdir(path.join(staging, '.cargo-target', 'release'), { recursive: true });
    // Exercise the packaging script's real filesystem copies without building Rust or Tauri.
    const script = (await readFile(new URL('tools/desktop.mjs', root), 'utf8'))
      .replace('import { spawn } from "node:child_process";', `import { EventEmitter } from 'node:events';
        const artifact = JSON.stringify({ reason: 'compiler-artifact', target: { name: 'viewer', kind: ['bin'] },
          executable: ${JSON.stringify(path.join(staging, '.cargo-target', 'release', 'viewer.exe'))} });
        const spawn = (_executable, _args, options) => {
          const child = new EventEmitter();
          if (Array.isArray(options.stdio)) child.stdout = Object.assign(new EventEmitter(), { setEncoding() { return this; } });
          queueMicrotask(() => { child.stdout?.emit('data', artifact + '\\n'); child.emit('exit', 0); child.emit('close', 0); });
          return child;
        };`)
      .replace(/const root = fileURLToPath\([^\n]+\);/, `const root = ${JSON.stringify(staging)};`)
      .replace('const mode = process.argv[2];', 'const mode = "build";');
    await import(`data:text/javascript;base64,${Buffer.from(script).toString('base64')}`);
    for (const font of fonts) {
      const notice = `LICENSES/Fontsource-${font}-LICENSE.txt`;
      assert.deepEqual(await readFile(path.join(staging, '.cargo-target', 'release', notice)),
        await readFile(new URL(notice, root)), `${font}: release carries the complete notice`);
    }
  } finally {
    assert.equal(path.dirname(path.resolve(staging)), path.resolve(tmpdir()));
    assert.ok(path.basename(staging).startsWith('replay-font-notices-'));
    await rm(staging, { recursive: true, force: true });
  }
});
