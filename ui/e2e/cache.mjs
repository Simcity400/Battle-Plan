import { mkdir, stat, readFile, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';
import { repository } from './settings.mjs';
const run = promisify(execFile);
export const readJSON = async file => JSON.parse(await readFile(file, 'utf8'));
export const writeJSON = async (file, data) => { await mkdir(path.dirname(file), { recursive: true }); await writeFile(file, JSON.stringify(data)); };
export function outsideRepository(directory) {
  const root = path.resolve(directory), inside = base => { const relative = path.relative(base, root); return !relative.startsWith('..') && !path.isAbsolute(relative); };
  // The checkout's own captures folder is ignored by Git.
  if (inside(repository) && !inside(path.join(repository, 'captures'))) throw Error('Keep captures outside Git');
  return root;
}
export async function validateDependencyLock(root) {
  const requested = (await readJSON(path.join(root, 'package-lock.json'))).packages ?? {};
  const installed = (await readJSON(path.join(root, 'node_modules/.package-lock.json'))).packages;
  if (!installed || Object.entries(installed).some(([name, value]) => value.version !== requested[name]?.version || value.integrity !== requested[name]?.integrity)
    || Object.entries(requested).some(([name, value]) => name && !value.optional && value.version !== installed[name]?.version))
    throw Error('UI dependency lock differs from installed dependencies');
}
export async function resolveUI(target) {
  const absolute = path.resolve(target);
  if (!(await stat(path.join(absolute, 'ui')).catch(() => null))?.isDirectory()) throw Error(`Prepared UI worktree required: ${target}`);
  await validateDependencyLock(absolute);
  return { root: absolute, commit: (await run('git', ['rev-parse', 'HEAD'], { cwd: absolute })).stdout.trim() };
}
