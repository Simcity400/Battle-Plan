// Cargo's `build.target` for a run from `cwd`, in Cargo's order: `--config` overrides (right-most
// first), CARGO_BUILD_TARGET, then each `.cargo` folder from `cwd` up and Cargo's home, where a
// folder's extensionless `config` hides its `config.toml`.
import { readFile } from 'node:fs/promises';
import path from 'node:path';

/** The target one TOML text names, undefined when it names none. */
function named(text, source) {
  let table = '';
  for (const line of text.split(/\r?\n/)) {
    const header = line.match(/^\s*\[\s*([^\]]+?)\s*\]/);
    if (header) { table = header[1]; continue; }
    const value = (table === 'build' ? line.match(/^\s*target\s*=\s*(.+)$/) : null)
      ?? (table === '' ? line.match(/^\s*build\.target\s*=\s*(.+)$/) : null);
    if (!value) continue;
    const target = value[1].match(/^"([^"]+)"/) ?? value[1].match(/^'([^']+)'/);
    if (!target) throw new Error(`${source} names build targets this command cannot place; pass --target.`);
    return target[1];
  }
  return undefined;
}

async function folderTarget(folder) {
  for (const name of ['config', 'config.toml']) {
    const file = path.join(folder, name);
    const text = await readFile(file, 'utf8').catch(() => null);
    if (text !== null) return named(text, file);
  }
  return undefined;
}

export async function cargoTarget({ cwd, home, env, cargoArgs }) {
  const overrides = [];
  for (const [index, arg] of cargoArgs.entries()) {
    if (arg === '--config' && index + 1 < cargoArgs.length) overrides.push(cargoArgs[index + 1]);
    else if (arg.startsWith('--config=')) overrides.push(arg.slice('--config='.length));
  }
  for (const override of overrides.reverse()) {
    const file = override.endsWith('.toml') ? path.resolve(cwd, override) : null;
    const target = file === null ? named(override, '--config') : named(await readFile(file, 'utf8'), file);
    if (target !== undefined) return target;
  }
  if (env.CARGO_BUILD_TARGET) return env.CARGO_BUILD_TARGET;
  for (let at = path.resolve(cwd); ; at = path.dirname(at)) {
    const target = await folderTarget(path.join(at, '.cargo'));
    if (target !== undefined) return target;
    if (path.dirname(at) === at) break;
  }
  return folderTarget(home);
}
