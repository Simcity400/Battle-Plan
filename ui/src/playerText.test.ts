import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { parseAst } from 'rolldown/parseAst';

// Text a player can see speaks about the campaign, never about how the recorder
// captured it. These phrases describe capture mechanics, internal data or UI
// tutorials; error messages (hidden from players as WidgetState details) and
// type-level strings are not player text.
const RECORDER_TALK: readonly RegExp[] = [
  /supplying read/i, /\bas read (at|on|earlier)\b/i, /\bread through\b/i, /\bread (date|hour)s?\b/i, /^Read$/, /\bfirst read\b/i,
  /\b(recorded|campaign['’]s|this|campaign) branch\b/i, /\bin this branch\b/i,
  /\bnative (country|countries|identit|holder|selection|action|event|result|frame|war|loss|bookkeeping|getter|operand)/i,
  /\bmalformed\b/i, /\boperands?\b/i, /\bprovenance\b/i, /\bcoverage (is|was) /i, /\bcoverage gaps?\b/i,
  /\bfirst observed\b/i, /\bobserved from\b/i, /\bobservation\b/i,
  /campaign['’]s own content/i, /\(not in this campaign\)/i, /not in the content files/i, /Gaps stay gaps/i,
  /a taller window/i, /^Click (for|to|a|the)\b/i, /\. Click (for|to|a|the)\b/i, /With Live on\b/i,
];

// Instructions for a control whose use is not obvious (a swatch you can click).
const EXPLAINS_A_CONTROL = new Set([
  'Click a theme to use it everywhere, or a swatch to use it only in light or dark mode.',
  'Click a swatch to change a faction’s colour',
]);

const root = fileURLToPath(new URL('.', import.meta.url));
// Rust-generated contracts (wire shapes, field paths, product names) never reach a player.
const generated = join(root, 'generated');
const ERROR_CALLEES = new Set(['Error', 'TypeError', 'RangeError']);
type Node = { type: string; start: number; [key: string]: unknown };

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return path === generated ? [] : sources(path);
    return /\.tsx?$/.test(entry.name) && !/\.(test|d)\.tsx?$/.test(entry.name) ? [path] : [];
  });
}

/** Whether a node never reaches a player: errors, logging, imports and types. */
function hidden(node: Node, parent: Node | undefined): boolean {
  if (node.type === 'ThrowStatement' || node.type.startsWith('TS') || node.type === 'ImportDeclaration'
    || node.type === 'ExportNamedDeclaration' && node.source || node.type === 'ExportAllDeclaration') return true;
  if (node.type === 'NewExpression' || node.type === 'CallExpression') {
    const callee = node.callee as Node & { name?: string; object?: { name?: string } };
    if (callee.type === 'Identifier' && ERROR_CALLEES.has(callee.name ?? '')) return true;
    if (callee.type === 'MemberExpression' && callee.object?.name === 'console') return true;
  }
  if (node.type === 'JSXAttribute' && (node.name as { name?: string }).name === 'className') return true;
  return parent?.type === 'Property' && parent.key === node && !parent.computed;
}

function playerTexts(code: string, lang: 'ts' | 'tsx'): { at: number; text: string }[] {
  const texts: { at: number; text: string }[] = [];
  const walk = (node: Node, parent?: Node) => {
    if (hidden(node, parent)) return;
    if (node.type === 'Literal' && typeof node.value === 'string') texts.push({ at: node.start, text: node.value });
    else if (node.type === 'TemplateElement') texts.push({ at: node.start, text: String((node.value as { cooked?: string }).cooked ?? '') });
    else if (node.type === 'JSXText') texts.push({ at: node.start, text: String(node.value).trim() });
    for (const value of Object.values(node)) {
      for (const child of Array.isArray(value) ? value : [value]) {
        if (child && typeof child === 'object' && typeof (child as Node).type === 'string') walk(child as Node, node);
      }
    }
  };
  walk(parseAst(code, { lang }) as unknown as Node);
  return texts;
}

test('player-facing text never talks about how the recording was captured', () => {
  const found: string[] = [];
  for (const path of sources(root)) {
    const code = readFileSync(path, 'utf8');
    for (const { at, text } of playerTexts(code, path.endsWith('x') ? 'tsx' : 'ts')) {
      if (EXPLAINS_A_CONTROL.has(text)) continue;
      if (RECORDER_TALK.some(pattern => pattern.test(text))) found.push(`${relative(root, path)}:${code.slice(0, at).split('\n').length}  ${JSON.stringify(text.slice(0, 120))}`);
    }
  }
  assert.deepEqual(found, [], `${found.length} player-facing strings use recorder language`);
});

test('the guard reads JSX text, attributes and templates, but not errors, logs, object keys or generated contracts', () => {
  const texts = playerTexts(`
    const a = <p className="observation" title="Click for more">Read through 1936 {x}</p>;
    const b = \`as read at \${hour}\`;
    throw new Error('The reply is malformed.');
    console.warn('native country');
    const c = { 'supplying read': 1 };`, 'tsx').map(item => item.text);
  assert.deepEqual(texts.filter(text => RECORDER_TALK.some(pattern => pattern.test(text))), ['Click for more', 'Read through 1936', 'as read at ']);
  const checked = sources(root).map(path => relative(root, path).replaceAll('\\', '/'));
  assert.ok(checked.includes('dashboard/CountryChip.tsx'), 'ordinary sources are checked');
  assert.ok(readdirSync(generated).some(name => name.endsWith('.ts')));
  assert.ok(!checked.some(path => path.startsWith('generated/')), 'generated contracts are skipped');
});
