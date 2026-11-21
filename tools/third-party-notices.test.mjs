import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { thirdPartyNotices } from './third-party-notices.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const licences = path.join(root, 'LICENSES');
const { notices } = JSON.parse(await readFile(path.join(root, 'tools', 'shipped.json'), 'utf8'));

test('every LICENSES file is in the combined notices once, verbatim, under its component in order', async () => {
  const combined = await thirdPartyNotices(licences, notices.components);
  let at = 0;
  for (const [index, component] of notices.components.entries()) {
    at = combined.indexOf(`${index + 1}. ${component.name}\n\n${component.use}\n`, at);
    assert.notEqual(at, -1, component.name);
    for (const name of component.files) {
      at = combined.indexOf(`--- ${name} ---\n\n${await readFile(path.join(licences, name), 'utf8')}`, at);
      assert.notEqual(at, -1, name);
    }
  }
  assert.deepEqual(notices.components.flatMap((component) => component.files).sort(), (await readdir(licences)).sort());
  assert.match(combined, /This software is based in part on the work of the Independent JPEG Group\./);
});

test('a notice missing from the manifest, listed without its file, nested or not UTF-8 stops packaging by name', async (t) => {
  const folder = await mkdtemp(path.join(tmpdir(), 'notices-'));
  t.after(() => rm(folder, { recursive: true, force: true }));
  await writeFile(path.join(folder, 'a.txt'), 'A licence');
  await writeFile(path.join(folder, 'new.txt'), 'Unlisted');
  await mkdir(path.join(folder, 'vendor'));
  await writeFile(path.join(folder, 'vendor', 'LICENSE'), 'Nested');
  await assert.rejects(thirdPartyNotices(folder, [{ name: 'A', use: 'Uses.', files: ['a.txt', 'gone.txt'] }]), (error) => {
    assert.match(error.message, /LICENSES\/vendor is not a file/);
    assert.match(error.message, /gone\.txt is listed but not in LICENSES/);
    assert.match(error.message, /LICENSES\/new\.txt is in no component/);
    return true;
  });
  await rm(path.join(folder, 'vendor'), { recursive: true });
  assert.match(await thirdPartyNotices(folder, [{ name: 'A', use: 'Uses.', files: ['a.txt', 'new.txt'] }]), /--- new\.txt ---\n\nUnlisted\n$/);
  await writeFile(path.join(folder, 'new.txt'), Buffer.from('Copyright \xa9', 'latin1'));
  await assert.rejects(thirdPartyNotices(folder, [{ name: 'A', use: 'Uses.', files: ['a.txt', 'new.txt'] }]), /LICENSES\/new\.txt is not UTF-8/);
});
