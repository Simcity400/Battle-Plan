import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { listReplayFolder } from '../../replayLibrary.ts';

test('replay library scans names only, includes subfolders and skips hidden folders and links', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'hoi4-library-'));
  try {
    const folder = path.join(root, 'replays');
    const outside = path.join(root, 'outside');
    await mkdir(path.join(folder, 'Campaign A'), { recursive: true });
    await mkdir(path.join(folder, '.hidden'));
    await mkdir(outside);
    await writeFile(path.join(folder, 'Campaign A', 'session.battleplan'), 'not parsed during listing');
    await writeFile(path.join(folder, 'second.battleplan'), '');
    await writeFile(path.join(folder, 'unfinished.battleplan.tmp'), '');
    await writeFile(path.join(folder, '.hidden', 'hidden.battleplan'), '');
    await writeFile(path.join(outside, 'external.battleplan'), '');
    await symlink(outside, path.join(folder, 'linked'), 'junction');
    const result = await listReplayFolder(folder);
    assert.equal(result.files.length, 2);
    assert.ok(result.files.some(file => file.name === 'session' && file.folder === 'Campaign A'));
    assert.ok(result.files.some(file => file.name === 'second' && file.folder === ''));
    const controller = new AbortController(); controller.abort();
    await assert.rejects(listReplayFolder(folder, controller.signal), { name: 'AbortError' });
    await assert.rejects(listReplayFolder(path.join(root, 'missing')));
  } finally {
    assert.equal(path.dirname(path.resolve(root)), path.resolve(tmpdir()));
    await rm(root, { recursive: true, force: true });
  }
});
