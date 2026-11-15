import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { cargoTarget } from './cargo-target.mjs';

async function fixture(files) {
  const root = await mkdtemp(path.join(tmpdir(), 'cargo-target-'));
  for (const [name, text] of Object.entries(files)) {
    await mkdir(path.dirname(path.join(root, name)), { recursive: true });
    await writeFile(path.join(root, name), text);
  }
  return root;
}

test("a folder's extensionless config wins over its config.toml, which Cargo then ignores", async (t) => {
  const root = await fixture({
    'work/.cargo/config': '[build]\njobs = 2\n',
    'work/.cargo/config.toml': '[build]\ntarget = "ignored-toml"\n',
    '.cargo/config.toml': 'build.target = "parent"\n',
    'home/config': '[build]\ntarget = "home"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  const resolve = (more = {}) => cargoTarget({ cwd: path.join(root, 'work', 'crate'), home: path.join(root, 'home'), env: {}, cargoArgs: [], ...more });
  assert.equal(await resolve(), 'parent', 'the extensionless file names no target, so the parent folder decides');
  await writeFile(path.join(root, 'work', '.cargo', 'config'), "[build]\ntarget = 'nearest'\n");
  assert.equal(await resolve(), 'nearest');
  await rm(path.join(root, '.cargo'), { recursive: true });
  await rm(path.join(root, 'work', '.cargo', 'config'));
  assert.equal(await resolve(), 'ignored-toml', 'alone, config.toml is read');
});

test('forwarded --config overrides beat the environment, which beats config files; the right-most override wins', async (t) => {
  const root = await fixture({
    '.cargo/config.toml': '[build]\ntarget = "file"\n',
    'extra.toml': '[build]\ntarget = "extra-file"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  const resolve = (env, cargoArgs) => cargoTarget({ cwd: root, home: path.join(root, 'no-home'), env, cargoArgs });
  assert.equal(await resolve({}, []), 'file');
  assert.equal(await resolve({ CARGO_BUILD_TARGET: 'env' }, []), 'env');
  assert.equal(await resolve({ CARGO_BUILD_TARGET: 'env' }, ['--config', 'build.target="cli"']), 'cli');
  assert.equal(await resolve({}, ['--config=build.target = "first"', '--release', '--config', "build.target='last'"]), 'last');
  assert.equal(await resolve({}, ['--config', 'build.target="cli"', '--config', 'build.jobs=2']), 'cli', 'a later override of another key leaves it');
  assert.equal(await resolve({ CARGO_BUILD_TARGET: 'env' }, ['--config', 'extra.toml']), 'extra-file', 'a config file named on the command line, from where Cargo runs');
  assert.equal(await resolve({}, ['--config', 'build.jobs=2']), 'file');
});

test('a list of targets is refused by name', async (t) => {
  const root = await fixture({ '.cargo/config.toml': '[build]\ntarget = ["a", "b"]\n' });
  t.after(() => rm(root, { recursive: true, force: true }));
  const home = path.join(root, 'no-home');
  await assert.rejects(cargoTarget({ cwd: root, home, env: {}, cargoArgs: [] }), /config\.toml names build targets this command cannot place/);
  await assert.rejects(cargoTarget({ cwd: root, home, env: {}, cargoArgs: ['--config', 'build.target=["a"]'] }), /--config names build targets/);
});
