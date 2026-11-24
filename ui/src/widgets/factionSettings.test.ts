import assert from 'node:assert/strict';
import { readdirSync, readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const WIDGETS = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.dirname(WIDGETS);
const READS = /\buse(FactionColours|FactionDisplayNames)\(|\.campaigns\[[^\]]+\]\??\.factions\b/;

function sources(folder: string): string[] {
  const seen = new Set<string>();
  const visit = (file: string) => {
    if (seen.has(file)) return;
    seen.add(file);
    for (const [, spec] of readFileSync(file, 'utf8').matchAll(/from '(\.[^']+)'/g)) {
      const base = path.resolve(path.dirname(file), spec);
      const found = [base, `${base}.tsx`, `${base}.ts`].find(candidate => existsSync(candidate) && /\.tsx?$/.test(candidate));
      if (found && (found.startsWith(folder) || found.startsWith(path.join(SRC, 'charts')))) visit(found);
    }
  };
  for (const name of readdirSync(folder)) if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) visit(path.join(folder, name));
  return [...seen];
}

test('every widget whose settings offer faction names and colours reads them, so a rename or colour reaches the widget', () => {
  const offering = readdirSync(WIDGETS).filter(name => existsSync(path.join(WIDGETS, name, 'definition.ts'))
    && /\bfactionColours:\s*true\b/.test(readFileSync(path.join(WIDGETS, name, 'definition.ts'), 'utf8')));
  assert.ok(offering.length > 0);
  const deaf = offering.filter(name => !sources(path.join(WIDGETS, name)).some(file => READS.test(readFileSync(file, 'utf8'))));
  assert.deepEqual(deaf, []);
});
