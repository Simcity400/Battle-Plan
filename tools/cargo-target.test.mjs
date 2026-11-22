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
  await assert.rejects(cargoTarget({ cwd: root, home, env: {}, cargoArgs: ['--config', '"build".target=["a", "b"]'] }), /--config names build targets/);
  await assert.rejects(cargoTarget({ cwd: root, home, env: {}, cargoArgs: ['--config', 'build.target=[]'] }), /--config names build targets/);
  await assert.rejects(cargoTarget({ cwd: root, home, env: {}, cargoArgs: ['--config', 'build.target=""'] }), /--config names build targets/);
  await assert.rejects(cargoTarget({ cwd: root, home, env: {}, cargoArgs: ['--config', 'build.target=123'] }), /--config names build targets/);
});

test('quoted, dotted, and escaped TOML keys and values are resolved across --config and config files', async (t) => {
  const root = await fixture({
    '.cargo/config.toml': '["build"]\n"target" = "quoted-table"\n',
    'escaped.toml': '"\\u0062uild"."t\\x61rget" = "targets\\\\x\\u00386_64\\U0000002dwin.json"\n',
    'literal.toml': "['build']\n'target' = 'C:\\targets\\custom.json'\n",
    'multiline.toml': '[build]\ntarget = """\nx86_64-\\\n  pc-windows-msvc"""\n',
    'multiline-literal.toml': "[build]\ntarget = '''\nC:\\targets\\multi.json'''\n",
    'custom-config': '[build]\ntarget = "extensionless-cli-file"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  const resolve = (cargoArgs = [], env = {}) => cargoTarget({ cwd: root, home: path.join(root, 'no-home'), env, cargoArgs });

  assert.equal(await resolve(), 'quoted-table');
  assert.equal(await resolve(['--config', '"build".target="quoted-cli"'], { CARGO_BUILD_TARGET: 'env' }), 'quoted-cli');
  assert.equal(await resolve(['--config', `'build'."target"='single-and-double'`]), 'single-and-double');
  assert.equal(await resolve(['--config', 'build . "target" = "spaced-dots"']), 'spaced-dots');
  assert.equal(await resolve(['--config', '"\\u0062uild"."\\u0074arget"="escaped-cli-key"']), 'escaped-cli-key');
  assert.equal(await resolve(['--config', 'build.target="dir\\\\custom\\"target\\".json"']), 'dir\\custom"target".json');
  assert.equal(await resolve(['--config', 'build.target="custom.toml"']), 'custom.toml');
  assert.equal(await resolve(['--config', 'escaped.toml']), 'targets\\x86_64-win.json');
  assert.equal(await resolve(['--config', 'literal.toml']), 'C:\\targets\\custom.json');
  assert.equal(await resolve(['--config', 'multiline.toml']), 'x86_64-pc-windows-msvc');
  assert.equal(await resolve(['--config', 'multiline-literal.toml']), 'C:\\targets\\multi.json');
  assert.equal(await resolve(['--config', 'custom-config']), 'extensionless-cli-file');
  assert.equal(await resolve(['--config', '"build".target="first"', '--config', '"build".jobs=2']), 'first');
  assert.equal(await resolve(['--config', 'build.target="first"', '--config', '"build".target="second"']), 'second');
});

test('inline tables, comments, real Cargo tables, and lookalike keys obey TOML structure', async (t) => {
  const root = await fixture({
    '.cargo/config.toml': [
      '\uFEFF[target.\'cfg(all(target_os = "windows", target_arch = "x86_64"))\']',
      'rustflags = ["-C", "link-arg=/DEBUG"] # trailing comment',
      '',
      '[other]',
      'build.target = "nested-other"',
      'nested = { build = { target = "nested-inline" } }',
      'note = """',
      '[build]',
      'target = "inside-string"',
      '"""',
      '""."build".target = "empty-prefix"',
      '',
      '[build.sub]',
      'target = "subtable-ignored"',
      '',
      '[build]',
      '# target = "commented-out"',
      'target = "real#not-a-comment" # comment after value',
    ].join('\n'),
    'inline.toml': 'build = { jobs = 4, "target" = "from-inline-table" }\n',
    'super-dotted.toml': '[build.sub.leaf]\nx = 1\n[build]\nsub.y = 2\ntarget = "after-dotted-sub"\n',
    'inline-array.toml': 'build = { target = [\n  # comment\n  "a",\n  "b",\n] }\n',
    'table-target.toml': '[build.target]\nfoo = "bar"\n',
    'scalar-build.toml': 'build = "not-a-table"\n',
    'aot-build.toml': '[[build]]\ntarget = "a"\n',
    'bad-duplicate.toml': '[build]\ntarget = "a"\ntarget = "b"\n',
    'bad-reopen.toml': 'build.jobs = 2\n[build]\ntarget = "a"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  const resolve = (cargoArgs = []) => cargoTarget({ cwd: root, home: path.join(root, 'no-home'), env: {}, cargoArgs });

  assert.equal(await resolve(), 'real#not-a-comment');
  assert.equal(await resolve(['--config', 'inline.toml']), 'from-inline-table');
  assert.equal(await resolve(['--config', 'super-dotted.toml']), 'after-dotted-sub');
  assert.equal(await resolve(['--config', 'build = { target = "cli-inline" }']), 'cli-inline');
  assert.equal(await resolve(['--config', 'other.build.target="ignored"', '--', '--config', 'build.target="after-sep"']), 'real#not-a-comment');
  await assert.rejects(resolve(['--config', 'inline-array.toml']), /inline-array\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'table-target.toml']), /table-target\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'scalar-build.toml']), /scalar-build\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'aot-build.toml']), /aot-build\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'missing.toml']), /--config/);
  await assert.rejects(resolve(['--config', 'bad-duplicate.toml']), /bad-duplicate\.toml/);
  await assert.rejects(resolve(['--config', 'bad-reopen.toml']), /bad-reopen\.toml/);
  await assert.rejects(resolve(['--config', '--config=build.target="injected"']), /--config/);
});
