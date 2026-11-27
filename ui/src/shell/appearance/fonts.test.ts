import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, statSync } from 'node:fs';
import { FONT_PRESETS, canUseFontFamily, fontStack, isMonospaceFamily } from './fonts.ts';

test('every named font preset ships its font faces and files with the app', () => {
  const entry = readFileSync(new URL('./bundledFonts.ts', import.meta.url), 'utf8');
  const sheets = [...entry.matchAll(/import\('(@fontsource-variable\/[^']+)\?inline'\)/g)].map(match => {
    const url = new URL(import.meta.resolve(match[1]));
    return { url, css: readFileSync(url, 'utf8') };
  });
  for (const preset of FONT_PRESETS.filter(preset => preset.family)) {
    const sheet = sheets.find(sheet => sheet.css.includes(`font-family: '${preset.family}';`));
    assert.ok(sheet, `${preset.label} must use a bundled font face`);
    const sources = [...sheet.css.matchAll(/src: url\(([^)]+)\)/g)];
    assert.ok(sources.length > 0, `${preset.label} must have font files`);
    for (const source of sources) {
      assert.ok(statSync(new URL(source[1], sheet.url)).size > 0, `${preset.label}: ${source[1]}`);
    }
  }
});

test('custom font names need a usable family while every preset works without installed fonts', () => {
  const before = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const context = { font: '', measureText: () => ({ width: 100 }) };
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { createElement: () => ({ getContext: () => context }) } });
  try {
    assert.equal(canUseFontFamily('Synthetic missing family'), false);
    assert.equal(canUseFontFamily('  Synthetic installed family  ', ['Synthetic installed family']), true);
    assert.equal(canUseFontFamily(''), true);
    for (const preset of FONT_PRESETS) assert.equal(canUseFontFamily(preset.family), true, preset.label);
  } finally {
    if (before) Object.defineProperty(globalThis, 'document', before);
    else Reflect.deleteProperty(globalThis, 'document');
  }
});

test('family stacks stay valid CSS and monospace roles reject proportional glyphs in any style', () => {
  assert.match(fontStack('Missing, "Arial", sans-serif'), /^'Missing', 'Arial', sans-serif,/);
  assert.equal(isMonospaceFamily(''), true);
  assert.equal(isMonospaceFamily('monospace'), true);
  assert.equal(isMonospaceFamily('Cascadia Code Variable'), true);
  assert.equal(isMonospaceFamily('Inter Variable'), false);
  const before = Object.getOwnPropertyDescriptor(globalThis, 'document');
  let proportionalStyle = '';
  const seen = new Set<string>();
  const context = { font: '', measureText: (glyph: string) => {
    seen.add(context.font);
    return { width: proportionalStyle && context.font.startsWith(proportionalStyle) && glyph === 'W' ? 30 : 20 };
  } };
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { createElement: () => ({ getContext: () => context }) } });
  try {
    assert.equal(isMonospaceFamily('Synthetic Mono'), true);
    assert.equal(seen.size, 4, 'regular, bold, italic and bold italic all checked');
    for (const style of ['400', '700', 'italic 400', 'italic 700']) {
      proportionalStyle = style;
      assert.equal(isMonospaceFamily('Synthetic Proportional'), false, style);
    }
  } finally {
    if (before) Object.defineProperty(globalThis, 'document', before);
    else Reflect.deleteProperty(globalThis, 'document');
  }
});
