import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { cargoTarget, parseHostTuple } from './cargo-target.mjs';
import { companionBuildArgs, metadataArgs, planDesktopInvocation, resolveTargetDir, stageOutputDir } from './desktop.mjs';

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
  assert.equal(await resolve(['--config', '"build".target="\\tx86_64-pc-windows-msvc"']), 'x86_64-pc-windows-msvc');
  await assert.rejects(resolve(['--config', '"build".target="  \\t  "']), /--config names build targets/);
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
  assert.equal(await resolve(['--config', 'other.build.target="ignored"', '--', '--config', 'build.target="after-sep"']), 'real#not-a-comment');
  assert.equal(await resolve(['--config', '  build.target = "ws-padded"  ']), 'ws-padded');
  await assert.rejects(resolve(['--config', 'build = { target = "cli-inline" }']), /--config/);
  await assert.rejects(resolve(['--config', '[build]\ntarget = "header-in-cli"']), /--config/);
  await assert.rejects(resolve(['--config', 'build.jobs = 1\nbuild.target = "two-stmts"']), /--config/);
  await assert.rejects(resolve(['--config', 'build.target = "with-comment" # nope']), /--config/);
  await assert.rejects(resolve(['--config', 'inline-array.toml']), /inline-array\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'table-target.toml']), /table-target\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'scalar-build.toml']), /scalar-build\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'aot-build.toml']), /aot-build\.toml names build targets this command cannot place/);
  await assert.rejects(resolve(['--config', 'missing.toml']), /--config/);
  await assert.rejects(resolve(['--config', 'bad-duplicate.toml']), /bad-duplicate\.toml/);
  await assert.rejects(resolve(['--config', 'bad-reopen.toml']), /bad-reopen\.toml/);
  await assert.rejects(resolve(['--config', '--config=build.target="injected"']), /--config/);
});

test('all config layers are validated and merged according to Cargo precedence rules', async (t) => {
  const root = await fixture({
    'arr/.cargo/config.toml': '[build]\ntarget = ["i686-pc-windows-msvc"]\n',
    'arr/child/.cargo/config.toml': '[build]\ntarget = "x86_64-pc-windows-msvc"\n',
    'scalar/.cargo/config.toml': '[build]\ntarget = "x86_64-pc-windows-msvc"\n',
    'int-file/.cargo/config.toml': '[build]\ntarget = 123\n',
    'bad-parent/.cargo/config.toml': '[broken\n',
    'bad-parent/child/.cargo/config.toml': '[build]\ntarget = "x86_64-pc-windows-msvc"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  const home = path.join(root, 'no-home');

  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'arr'), home, env: { CARGO_BUILD_TARGET: 'x86_64-pc-windows-msvc' }, cargoArgs: [] }),
    /names build targets this command cannot place/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'arr', 'child'), home, env: {}, cargoArgs: [] }),
    /names build targets this command cannot place/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'arr'), home, env: {}, cargoArgs: ['--config', 'build.target="x86_64-pc-windows-msvc"'] }),
    /names build targets this command cannot place/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'scalar'), home, env: {}, cargoArgs: ['--config', 'build.target=["x86_64-pc-windows-msvc"]'] }),
    /names build targets this command cannot place/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'scalar'), home, env: { CARGO_BUILD_TARGET: '' }, cargoArgs: [] }),
    /CARGO_BUILD_TARGET names build targets/,
  );
  assert.equal(
    await cargoTarget({ cwd: path.join(root, 'scalar'), home, env: { CARGO_BUILD_TARGET: '' }, cargoArgs: ['--config', 'build.target="cli-override"'] }),
    'cli-override',
  );
  assert.equal(
    await cargoTarget({ cwd: path.join(root, 'scalar'), home, env: {}, cargoArgs: ['--config', 'build.target=123', '--config', '"build".target="x86_64-pc-windows-msvc"'] }),
    'x86_64-pc-windows-msvc',
  );
  assert.equal(
    await cargoTarget({ cwd: path.join(root, 'scalar'), home, env: {}, cargoArgs: ['--config', 'build.target=true', '--config', 'build.target="x86_64-pc-windows-msvc"'] }),
    'x86_64-pc-windows-msvc',
  );
  assert.equal(
    await cargoTarget({ cwd: path.join(root, 'int-file'), home, env: { CARGO_BUILD_TARGET: 'x86_64-pc-windows-msvc' }, cargoArgs: [] }),
    'x86_64-pc-windows-msvc',
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'int-file'), home, env: {}, cargoArgs: [] }),
    /config\.toml names build targets this command cannot place/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'scalar'), home, env: {}, cargoArgs: ['--config', 'build.target=123', '--config', 'build.target=["x"]'] }),
    /names build targets this command cannot place/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'scalar'), home, env: {}, cargoArgs: ['--config', 'build.target=["a"]', '--config', 'build.target="b"'] }),
    /names build targets this command cannot place/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'scalar'), home, env: {}, cargoArgs: ['--config', 'not-valid-toml', '--config', 'build.target="b"'] }),
    /--config/,
  );
  await assert.rejects(
    cargoTarget({ cwd: path.join(root, 'bad-parent', 'child'), home, env: {}, cargoArgs: [] }),
    /bad-parent/,
  );
});

test('Cargo include directives resolve recursively with precedence, optionality, .toml requirement, and duplicate-visit refusal', async (t) => {
  const root = await fixture({
    'basic/.cargo/windows.toml': '[build]\ntarget = "from-include"\n',
    'basic/.cargo/config.toml': 'include = ["windows.toml"]\n',
    'order/.cargo/a.toml': '[build]\ntarget = "from-a"\n',
    'order/.cargo/b.toml': '[build]\ntarget = "from-b"\n',
    'order/.cargo/config.toml': 'include = ["a.toml", { path = "b.toml", optional = true }]\n',
    'override/.cargo/a.toml': '[build]\ntarget = "from-a"\n',
    'override/.cargo/config.toml': 'include = ["a.toml"]\n[build]\ntarget = "from-main"\n',
    'nested/.cargo/sub/leaf.toml': '[build]\ntarget = "from-nested"\n',
    'nested/.cargo/sub/mid.toml': 'include = ["leaf.toml"]\n',
    'nested/.cargo/config.toml': 'include = ["sub/mid.toml"]\n',
    'opt-missing/.cargo/config.toml': 'include = [{ path = "absent.toml", optional = true }]\n',
    'opt-nested-missing/.cargo/inner.toml': 'include = ["really-missing.toml"]\n',
    'opt-nested-missing/.cargo/config.toml': 'include = [{ path = "inner.toml", optional = true }]\n',
    'opt-bad/.cargo/broken.toml': '[build\n',
    'opt-bad/.cargo/config.toml': 'include = [{ path = "broken.toml", optional = true }]\n',
    'req-missing/.cargo/config.toml': 'include = ["absent.toml"]\n',
    'bad-string/.cargo/config.toml': 'include = "windows.toml"\n',
    'bad-optional/.cargo/config.toml': 'include = [{ path = "a.toml", optional = "yes" }]\n',
    'no-ext/.cargo/windows': '[build]\ntarget = "from-noext"\n',
    'no-ext/.cargo/config.toml': 'include = ["windows"]\n',
    'no-ext-opt/.cargo/config.toml': 'include = [{ path = "windows", optional = true }]\n',
    'cycle/.cargo/a.toml': 'include = ["b.toml"]\n',
    'cycle/.cargo/b.toml': 'include = ["a.toml"]\n',
    'cycle/.cargo/config.toml': 'include = ["a.toml"]\n',
    'diamond/.cargo/shared.toml': '[build]\ntarget = "shared"\n',
    'diamond/.cargo/a.toml': 'include = ["shared.toml"]\n',
    'diamond/.cargo/b.toml': 'include = ["shared.toml"]\n',
    'diamond/.cargo/config.toml': 'include = ["a.toml", "b.toml"]\n',
    'shared-layers/.cargo/shared.toml': '[build]\ntarget = "shared-between-layers"\n',
    'shared-layers/.cargo/config.toml': 'include = ["shared.toml"]\n',
    'shared-layers/child/.cargo/config.toml': 'include = ["../../.cargo/shared.toml"]\n',
    'cli-inc/extra.toml': '[build]\ntarget = "from-cli-include"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  const home = path.join(root, 'no-home');
  const resolveAt = (dir, cargoArgs = []) => cargoTarget({ cwd: path.join(root, dir), home, env: {}, cargoArgs });

  assert.equal(await resolveAt('basic'), 'from-include');
  assert.equal(await resolveAt('order'), 'from-b');
  assert.equal(await resolveAt('override'), 'from-main');
  assert.equal(await resolveAt('nested'), 'from-nested');
  assert.equal(await resolveAt('opt-missing'), undefined);
  assert.equal(await resolveAt('shared-layers/child'), 'shared-between-layers');
  assert.equal(await resolveAt('cli-inc', ['--config', 'include=["extra.toml"]']), 'from-cli-include');
  await assert.rejects(resolveAt('diamond'), /shared\.toml/);
  await assert.rejects(resolveAt('no-ext'), /\.toml/);
  await assert.rejects(resolveAt('no-ext-opt'), /\.toml/);
  await assert.rejects(resolveAt('opt-nested-missing'), /really-missing\.toml/);
  await assert.rejects(resolveAt('opt-bad'), /broken\.toml/);
  await assert.rejects(resolveAt('req-missing'), /absent\.toml/);
  await assert.rejects(resolveAt('bad-string'), /include/);
  await assert.rejects(resolveAt('bad-optional'), /optional/);
  await assert.rejects(resolveAt('cycle'), /a\.toml/);
});

test('strict UTF-8 decoding and TOML date/time refusal match Cargo config loading', async (t) => {
  const root = await fixture({
    'offset-datetime/.cargo/config.toml': 'stamp = 2024-02-29T23:59:60-07:00\n[build]\ntarget = "ok"\n',
    'local-date/.cargo/config.toml': 'stamp = 2024-02-29\n[build]\ntarget = "ok"\n',
    'local-time/.cargo/config.toml': 'stamp = 12:30:00\n[build]\ntarget = "ok"\n',
    'bad-month/.cargo/config.toml': 'stamp = 2026-99-99\n[build]\ntarget = "ok"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, 'bad-utf8', '.cargo'), { recursive: true });
  await writeFile(path.join(root, 'bad-utf8', '.cargo', 'config.toml'), Buffer.from([0x5b, 0x62, 0x75, 0x69, 0x6c, 0x64, 0x5d, 0x0a, 0xff, 0xfe]));
  const home = path.join(root, 'no-home');
  const resolveAt = (dir) => cargoTarget({ cwd: path.join(root, dir), home, env: {}, cargoArgs: [] });

  await assert.rejects(resolveAt('offset-datetime'), /Invalid TOML/);
  await assert.rejects(resolveAt('local-date'), /Invalid TOML/);
  await assert.rejects(resolveAt('local-time'), /Invalid TOML/);
  await assert.rejects(resolveAt('bad-month'), /Invalid TOML/);
  await assert.rejects(resolveAt('bad-utf8'), /UTF-8|utf-8/i);
});

test('desktop staging forwards targets to Tauri and companions, forwards --config to metadata, expands host-tuple, and refuses JSON specs', async (t) => {
  const root = await fixture({
    'plain/crates/desktop/Cargo.toml': '[package]\nname = "desktop"\n',
    'desktop-cfg/crates/desktop/.cargo/config.toml': '[build]\ntarget = "aarch64-pc-windows-msvc"\n',
    'rel-home/crates/desktop/Cargo.toml': '[package]\nname = "desktop"\n',
    'rel-home/crates/desktop/cargo-home/config.toml': '[build]\ntarget = "aarch64-pc-windows-msvc"\n',
    'int-workspace/.cargo/config.toml': '[build]\ntarget = 123\n',
    'int-workspace/crates/desktop/Cargo.toml': '[package]\nname = "desktop"\n',
    'int-include/.cargo/config.toml': '[build]\ntarget = 123\n',
    'int-include/crates/desktop/.cargo/override.toml': '[build]\ntarget = "x86_64-pc-windows-msvc"\n',
    'int-include/crates/desktop/.cargo/config.toml': 'include = ["override.toml"]\n',
    'int-include/crates/desktop/Cargo.toml': '[package]\nname = "desktop"\n',
    'json-desktop/crates/desktop/.cargo/config.toml': '[build]\ntarget = "targets/custom.json"\n',
    'json-root/.cargo/config.toml': '[build]\ntarget = "targets/root-spec.json"\n',
    'json-root/crates/desktop/Cargo.toml': '[package]\nname = "desktop"\n',
    'multi/.cargo/config.toml': '[build]\ntarget = ["a", "b"]\n',
    'multi/crates/desktop/Cargo.toml': '[package]\nname = "desktop"\n',
  });
  t.after(() => rm(root, { recursive: true, force: true }));
  const env = { CARGO_HOME: path.join(root, 'no-home') };
  const hostTarget = async () => 'x86_64-pc-windows-msvc';
  const embeds = new Set(['recorder']);

  assert.equal(parseHostTuple('rustc 1.96.0\nhost: x86_64-pc-windows-msvc\nrelease: 1.96.0\n'), 'x86_64-pc-windows-msvc');
  assert.throws(() => parseHostTuple('rustc 1.96.0\nrelease: 1.96.0\n'), /host target triple/);

  const cliConfigPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['--', '--config', '"build".target="aarch64-pc-windows-msvc"', '--config=build.target-dir="alt-target"'],
    root: path.join(root, 'plain'),
    env,
    hostTarget,
  });
  assert.deepEqual(cliConfigPlan.targetArgs, ['--target', 'aarch64-pc-windows-msvc']);
  assert.equal(cliConfigPlan.outputDirName, 'aarch64-pc-windows-msvc');
  assert.deepEqual(cliConfigPlan.tauriArgs, [
    '--target', 'aarch64-pc-windows-msvc',
    '--',
    '--profile', 'release-lto',
    '--config', '"build".target="aarch64-pc-windows-msvc"',
    '--config=build.target-dir="alt-target"',
  ]);
  assert.equal(cliConfigPlan.metadataCwd, path.join(root, 'plain', 'crates', 'desktop'));
  assert.equal(cliConfigPlan.cargoCwd, path.join(root, 'plain', 'crates', 'desktop'));
  assert.deepEqual(cliConfigPlan.configArgs, [
    '--config', '"build".target="aarch64-pc-windows-msvc"',
    '--config', 'build.target-dir="alt-target"',
  ]);
  assert.deepEqual(metadataArgs(cliConfigPlan), [
    'metadata', '--no-deps', '--format-version', '1',
    '--config', '"build".target="aarch64-pc-windows-msvc"',
    '--config', 'build.target-dir="alt-target"',
  ]);
  assert.equal(resolveTargetDir(cliConfigPlan, path.join(root, 'from-metadata')), path.join(root, 'from-metadata'));
  const explicitTargetDirPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['--', '--target-dir', 'custom-out', '--config', 'build.target-dir="ignored-by-flag"'],
    root: path.join(root, 'plain'),
    env,
    hostTarget,
  });
  assert.equal(resolveTargetDir(explicitTargetDirPlan, path.join(root, 'from-metadata')), path.join(root, 'plain', 'custom-out'));
  assert.deepEqual(
    companionBuildArgs({
      name: 'recorder',
      embeds,
      profile: cliConfigPlan.profile,
      targetArgs: cliConfigPlan.targetArgs,
      targetDirArgs: ['--target-dir', path.join(root, 'alt')],
      configArgs: cliConfigPlan.configArgs,
    }),
    [
      'build', '-p', 'recorder', '--features', 'recorder/custom-protocol', '--profile', 'release-lto',
      '--target', 'aarch64-pc-windows-msvc', '--target-dir', path.join(root, 'alt'),
      '--config', '"build".target="aarch64-pc-windows-msvc"', '--config', 'build.target-dir="alt-target"',
      '--message-format=json-render-diagnostics',
    ],
  );
  assert.equal(
    stageOutputDir({ targetDir: path.join(root, 'out'), outputDirName: cliConfigPlan.outputDirName, profile: cliConfigPlan.profile }),
    path.join(root, 'out', 'aarch64-pc-windows-msvc', 'release-lto'),
  );

  const intOverridePlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['--', '--config', '"build".target="\\tx86_64-pc-windows-msvc"'],
    root: path.join(root, 'int-workspace'),
    env,
    hostTarget,
  });
  assert.deepEqual(intOverridePlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.equal(intOverridePlan.outputDirName, 'x86_64-pc-windows-msvc');
  assert.deepEqual(intOverridePlan.tauriEnv, { CARGO_BUILD_TARGET: 'x86_64-pc-windows-msvc' });
  assert.deepEqual(intOverridePlan.tauriArgs, [
    '--target', 'x86_64-pc-windows-msvc',
    '--',
    '--profile', 'release-lto',
    '--config', '"build".target="\\tx86_64-pc-windows-msvc"',
  ]);
  assert.deepEqual(
    companionBuildArgs({
      name: 'collector',
      embeds,
      profile: intOverridePlan.profile,
      targetArgs: intOverridePlan.targetArgs,
      targetDirArgs: ['--target-dir', path.join(root, 'out')],
      configArgs: intOverridePlan.configArgs,
    }),
    [
      'build', '-p', 'collector', '--profile', 'release-lto',
      '--target', 'x86_64-pc-windows-msvc', '--target-dir', path.join(root, 'out'),
      '--config', '"build".target="\\tx86_64-pc-windows-msvc"',
      '--message-format=json-render-diagnostics',
    ],
  );

  const intOverrideWithExplicitPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['-tx86_64-pc-windows-msvc', '--', '--config', '"build".target="x86_64-pc-windows-msvc"'],
    root: path.join(root, 'int-workspace'),
    env,
    hostTarget,
  });
  assert.deepEqual(intOverrideWithExplicitPlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.deepEqual(intOverrideWithExplicitPlan.tauriEnv, { CARGO_BUILD_TARGET: 'x86_64-pc-windows-msvc' });
  assert.deepEqual(intOverrideWithExplicitPlan.tauriArgs, [
    '--target', 'x86_64-pc-windows-msvc',
    '--',
    '--profile', 'release-lto',
    '--config', '"build".target="x86_64-pc-windows-msvc"',
  ]);

  const multiWithUnrelatedConfigAndExplicitPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['-tx86_64-pc-windows-msvc', '--', '--config', 'build.jobs=2'],
    root: path.join(root, 'multi'),
    env,
    hostTarget,
  });
  assert.deepEqual(multiWithUnrelatedConfigAndExplicitPlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.deepEqual(multiWithUnrelatedConfigAndExplicitPlan.tauriEnv, {});

  const groupedShortPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['-dtx86_64-pc-windows-msvc'],
    root: path.join(root, 'multi'),
    env,
    hostTarget,
  });
  assert.equal(groupedShortPlan.profile, 'dev');
  assert.deepEqual(groupedShortPlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.deepEqual(groupedShortPlan.tauriArgs, ['-d', '--target', 'x86_64-pc-windows-msvc']);
  assert.deepEqual(groupedShortPlan.tauriEnv, {});

  const relHomePlan = await planDesktopInvocation({
    mode: 'dev',
    argv: [],
    root: path.join(root, 'rel-home'),
    env: { CARGO_HOME: 'cargo-home' },
    hostTarget,
  });
  assert.equal(
    await cargoTarget({ cwd: path.join(root, 'rel-home', 'crates', 'desktop'), home: 'cargo-home', env: {}, cargoArgs: [] }),
    'aarch64-pc-windows-msvc',
  );
  assert.deepEqual(relHomePlan.targetArgs, ['--target', 'aarch64-pc-windows-msvc']);
  assert.deepEqual(relHomePlan.tauriArgs, ['--target', 'aarch64-pc-windows-msvc', '--']);

  const intIncludePlan = await planDesktopInvocation({
    mode: 'build',
    argv: [],
    root: path.join(root, 'int-include'),
    env,
    hostTarget,
  });
  assert.deepEqual(intIncludePlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.deepEqual(intIncludePlan.tauriArgs, ['--target', 'x86_64-pc-windows-msvc', '--', '--profile', 'release-lto']);
  assert.deepEqual(intIncludePlan.tauriEnv, { CARGO_BUILD_TARGET: 'x86_64-pc-windows-msvc' });

  const intIncludeWithExplicitPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['-tx86_64-pc-windows-msvc'],
    root: path.join(root, 'int-include'),
    env,
    hostTarget,
  });
  assert.deepEqual(intIncludeWithExplicitPlan.tauriEnv, { CARGO_BUILD_TARGET: 'x86_64-pc-windows-msvc' });

  const desktopCfgPlan = await planDesktopInvocation({
    mode: 'build',
    argv: [],
    root: path.join(root, 'desktop-cfg'),
    env,
    hostTarget,
  });
  assert.deepEqual(desktopCfgPlan.targetArgs, ['--target', 'aarch64-pc-windows-msvc']);
  assert.deepEqual(desktopCfgPlan.tauriArgs, ['--target', 'aarch64-pc-windows-msvc', '--', '--profile', 'release-lto']);
  assert.deepEqual(desktopCfgPlan.tauriEnv, { CARGO_BUILD_TARGET: 'aarch64-pc-windows-msvc' });

  const envTargetPlan = await planDesktopInvocation({
    mode: 'dev',
    argv: [],
    root: path.join(root, 'plain'),
    env: { ...env, CARGO_BUILD_TARGET: 'aarch64-pc-windows-msvc' },
    hostTarget,
  });
  assert.deepEqual(envTargetPlan.tauriArgs, ['--target', 'aarch64-pc-windows-msvc', '--']);

  const cargoTargetAfterSepPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['--', '--target', 'aarch64-pc-windows-msvc', '--jobs', '4'],
    root: path.join(root, 'plain'),
    env,
    hostTarget,
  });
  assert.deepEqual(cargoTargetAfterSepPlan.tauriArgs, ['--target', 'aarch64-pc-windows-msvc', '--', '--profile', 'release-lto', '--jobs', '4']);

  const hostTuplePlan = await planDesktopInvocation({
    mode: 'dev',
    argv: ['--', '--config', '"build".target="host-tuple"'],
    root: path.join(root, 'plain'),
    env,
    hostTarget,
  });
  assert.deepEqual(hostTuplePlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.equal(hostTuplePlan.outputDirName, 'x86_64-pc-windows-msvc');
  assert.deepEqual(hostTuplePlan.tauriArgs, ['--target', 'x86_64-pc-windows-msvc', '--', '--config', '"build".target="host-tuple"']);
  assert.equal(
    stageOutputDir({ targetDir: path.join(root, 'out'), outputDirName: hostTuplePlan.outputDirName, profile: hostTuplePlan.profile }),
    path.join(root, 'out', 'x86_64-pc-windows-msvc', 'debug'),
  );

  const explicitHostTuplePlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['--target', 'host-tuple'],
    root: path.join(root, 'plain'),
    env,
    hostTarget,
  });
  assert.deepEqual(explicitHostTuplePlan.tauriArgs, ['--target', 'x86_64-pc-windows-msvc', '--', '--profile', 'release-lto']);

  const attachedShortPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['-tx86_64-pc-windows-msvc'],
    root: path.join(root, 'multi'),
    env,
    hostTarget,
  });
  assert.deepEqual(attachedShortPlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.deepEqual(attachedShortPlan.tauriArgs, ['--target', 'x86_64-pc-windows-msvc', '--', '--profile', 'release-lto']);
  assert.deepEqual(attachedShortPlan.tauriEnv, {});

  const attachedEqShortPlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['-t=x86_64-pc-windows-msvc'],
    root: path.join(root, 'multi'),
    env,
    hostTarget,
  });
  assert.deepEqual(attachedEqShortPlan.targetArgs, ['--target', 'x86_64-pc-windows-msvc']);
  assert.deepEqual(attachedEqShortPlan.tauriArgs, ['--target', 'x86_64-pc-windows-msvc', '--', '--profile', 'release-lto']);
  assert.deepEqual(attachedEqShortPlan.tauriEnv, {});

  const attachedHostTuplePlan = await planDesktopInvocation({
    mode: 'build',
    argv: ['-thost-tuple'],
    root: path.join(root, 'multi'),
    env,
    hostTarget,
  });
  assert.deepEqual(attachedHostTuplePlan.tauriArgs, ['--target', 'x86_64-pc-windows-msvc', '--', '--profile', 'release-lto']);

  const explicitOverridesMulti = await planDesktopInvocation({
    mode: 'build',
    argv: ['--target', 'aarch64-pc-windows-msvc'],
    root: path.join(root, 'multi'),
    env,
    hostTarget,
  });
  assert.deepEqual(explicitOverridesMulti.targetArgs, ['--target', 'aarch64-pc-windows-msvc']);
  assert.deepEqual(explicitOverridesMulti.tauriArgs, ['--target', 'aarch64-pc-windows-msvc', '--', '--profile', 'release-lto']);
  assert.deepEqual(explicitOverridesMulti.tauriEnv, {});

  await assert.rejects(
    planDesktopInvocation({ mode: 'build', argv: ['-t=a', '-tb'], root: path.join(root, 'plain'), env, hostTarget }),
    /multiple build targets/,
  );

  await assert.rejects(
    planDesktopInvocation({ mode: 'build', argv: [], root: path.join(root, 'json-desktop'), env, hostTarget }),
    /JSON target spec/i,
  );
  await assert.rejects(
    planDesktopInvocation({ mode: 'dev', argv: [], root: path.join(root, 'json-root'), env, hostTarget }),
    /JSON target spec/i,
  );
  await assert.rejects(
    planDesktopInvocation({ mode: 'dev', argv: ['--', '--config', 'build.target="specs/my.custom.json"'], root: path.join(root, 'plain'), env, hostTarget }),
    /JSON target spec/i,
  );
  await assert.rejects(
    planDesktopInvocation({ mode: 'build', argv: ['--target', 'specs/custom.json'], root: path.join(root, 'plain'), env, hostTarget }),
    /JSON target spec/i,
  );
  await assert.rejects(
    planDesktopInvocation({ mode: 'build', argv: ['--target', 'a', '--target', 'b'], root: path.join(root, 'plain'), env, hostTarget }),
    /multiple build targets/,
  );
});
