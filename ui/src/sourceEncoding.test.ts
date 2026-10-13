import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

// The production build reads sources as UTF-8 and refuses any other bytes; the
// development server and the type check do not notice them.
const root = fileURLToPath(new URL('..', import.meta.url));
const SOURCES = new Set(['.ts', '.tsx', '.css', '.html', '.json', '.md']);

function* files(directory: string): Generator<string> {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'node_modules' && entry.name !== 'dist') yield* files(path);
    } else if (SOURCES.has(extname(entry.name))) yield path;
  }
}

test('every frontend source is UTF-8, as the production build requires', () => {
  const decoder = new TextDecoder('utf-8', { fatal: true });
  const invalid = [...files(root)].filter(path => {
    try { decoder.decode(readFileSync(path)); return false; } catch { return true; }
  }).map(path => relative(root, path));
  assert.deepEqual(invalid, []);
});
