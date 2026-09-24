import test from 'node:test';
import assert from 'node:assert/strict';
import { contrast, normalizeColor, oklchToRgb, parseColor, parseColorAlpha, rgbToOklch, toHex } from './color.ts';
import { BUILT_IN_THEMES, contrastWarnings, cssVariable, deriveColors, themeRoles } from './themes.ts';
import { parseJsonc, parseThemeText, serializeTheme, uniqueThemeName } from './themeFile.ts';
import { APPEARANCE_KEY, DEFAULT_APPEARANCE, loadAppearance, migrateLegacy, parseAppearance, resolveAppearance, saveAppearance } from './settings.ts';

const hex = /^#[0-9a-f]{6}$/;
function memoryStorage(entries: Record<string, string> = {}) {
  const values = new Map(Object.entries(entries));
  return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
}

test('colours parse from hex and rgb forms and composite translucency', () => {
  assert.deepEqual(parseColor('#fff'), { r: 255, g: 255, b: 255 });
  assert.deepEqual(parseColor('#0A141E'), { r: 10, g: 20, b: 30 });
  assert.deepEqual(parseColorAlpha('#00000080')?.alpha.toFixed(2), '0.50');
  assert.deepEqual(parseColor('rgb(10 20 30)'), { r: 10, g: 20, b: 30 });
  assert.deepEqual(parseColor('rgba(10, 20, 30, 0.4)'), { r: 10, g: 20, b: 30 });
  assert.equal(parseColor('red'), null);
  assert.equal(parseColor('#12345'), null);
  assert.equal(normalizeColor('#ffffff80', '#000000'), '#808080');
  assert.equal(normalizeColor('  #ABCDEF '), '#abcdef');
});

test('OKLCH conversion round-trips sRGB colours', () => {
  for (const value of ['#000000', '#ffffff', '#5cb2ff', '#c2410c', '#0a0f16', '#aac77e', '#ff00ff']) {
    assert.equal(toHex(oklchToRgb(rgbToOklch(parseColor(value)!))), value);
  }
  // Out-of-gamut requests keep their lightness by losing chroma.
  const clipped = oklchToRgb({ l: 0.7, c: 0.5, h: 140 });
  assert.ok([clipped.r, clipped.g, clipped.b].every(channel => channel >= 0 && channel <= 255));
});

test('derived palettes are complete and readable for dark and light canvases', () => {
  for (const seeds of [{ canvas: '#0a0f16', accent: '#5cb2ff' }, { canvas: '#f1ebdf', accent: '#95650f' }, { canvas: '#301934', accent: '#ffd700' }]) {
    const colors = deriveColors(seeds);
    assert.deepEqual(Object.keys(colors).sort(), [...themeRoles].sort());
    for (const role of themeRoles) assert.match(colors[role], hex, role);
    assert.ok(contrast(parseColor(colors.text)!, parseColor(colors.canvas)!) >= 7);
    assert.ok(contrast(parseColor(colors.muted)!, parseColor(colors.surface)!) >= 4.5);
    assert.ok(contrast(parseColor(colors.accentText)!, parseColor(colors.accent)!) >= 3);
  }
});

test('every built-in theme variant passes the editor contrast checks', () => {
  for (const theme of BUILT_IN_THEMES) {
    for (const mode of ['dark', 'light'] as const) {
      const colors = theme.variants[mode]!;
      assert.deepEqual(contrastWarnings(colors), [], `${theme.name} ${mode}`);
    }
  }
  assert.equal(new Set(BUILT_IN_THEMES.map(theme => theme.id)).size, BUILT_IN_THEMES.length);
  assert.equal(cssVariable('accentText'), '--accent-text');
});

test('theme files round-trip and accept partial palettes', () => {
  const source = { name: 'Round trip', variants: { dark: BUILT_IN_THEMES[1].variants.dark!, light: BUILT_IN_THEMES[2].variants.light! } };
  const parsed = parseThemeText(serializeTheme(source));
  assert.equal(parsed.name, 'Round trip');
  assert.deepEqual(parsed.variants, source.variants);
  assert.deepEqual(parsed.notes, []);

  const partial = parseThemeText(JSON.stringify({ name: 'Two colours', appearance: 'light', canvas: '#f4f1ea', accent: '#8a3ffc', sparkle: '#fff' }));
  assert.equal(partial.variants.dark, undefined);
  assert.equal(partial.variants.light?.canvas, '#f4f1ea');
  assert.equal(partial.variants.light?.accent, '#8a3ffc');
  assert.match(partial.variants.light!.muted, hex);

  const noted = parseThemeText(JSON.stringify({ variants: { dark: { canvas: '#101010', accent: '#ff0000', text: 'nope', typo: '#fff' } } }));
  assert.equal(noted.name, 'Imported theme');
  assert.equal(noted.notes.length, 2);
  assert.notEqual(noted.variants.dark!.text, 'nope');
});

test('theme import rejects unreadable files', () => {
  assert.throws(() => parseThemeText('not json'), /valid theme/);
  assert.throws(() => parseThemeText('[]'), /valid theme/);
  assert.throws(() => parseThemeText(JSON.stringify({ name: 'x', canvas: 'blue' })), /no readable/);
  assert.throws(() => parseThemeText(JSON.stringify({ format: 'other', canvas: '#000' })), /not an Battle Plan theme/);
  assert.throws(() => parseThemeText(JSON.stringify({ version: 9, canvas: '#000' })), /version 9/);
  assert.throws(() => parseThemeText('x'.repeat(600 * 1024)), /512 KB/);
});

test('VS Code themes with comments and trailing commas convert to one variant', () => {
  const text = `// Night owl-ish
  {
    "name": "Harbour Night",
    "type": "dark",
    /* workbench */
    "colors": {
      "editor.background": "#011627",
      "editor.foreground": "#d6deeb",
      "button.background": "#7e57c2cc", // translucent
      "panel.border": "#5f7e97",
      "terminal.ansiGreen": "#22da6e",
    },
    "tokenColors": [{ "scope": "comment", "settings": { "foreground": "#637777" } },],
  }`;
  assert.deepEqual(parseJsonc('{"a": "//not a comment", /* x */ "b": [1,],}'), { a: '//not a comment', b: [1] });
  // Commas before a closing bracket inside a string are text, not trailing commas.
  assert.deepEqual(parseJsonc('{"a": "x, }", "b": "y,\\"]",}'), { a: 'x, }', b: 'y,"]' });
  // A trailing comma may be followed by comments and whitespace; a kept comma may not be dropped.
  assert.deepEqual(parseJsonc('[1, // one\n /* two */ ]'), [1]);
  assert.deepEqual(parseJsonc('[1, /* x */ 2]'), [1, 2]);
  // Linear in size: a 512 KB theme of many closing brackets parses quickly.
  const rules = Array.from({ length: 8000 }, (_, i) => `{"scope": "s${i}", "settings": {"foreground": "#112233",},},`).join('');
  const started = performance.now();
  assert.equal((parseJsonc(`{"tokenColors": [${rules}]}`) as { tokenColors: unknown[] }).tokenColors.length, 8000);
  assert.ok(performance.now() - started < 250, 'parseJsonc must stay linear');
  const imported = parseThemeText(text);
  assert.equal(imported.source, 'vscode');
  assert.equal(imported.name, 'Harbour Night');
  assert.equal(imported.variants.light, undefined);
  const colors = imported.variants.dark!;
  assert.equal(colors.canvas, '#011627');
  assert.equal(colors.text, '#d6deeb');
  assert.equal(colors.border, '#5f7e97');
  assert.equal(colors.positive, '#22da6e');
  assert.equal(colors.accent, normalizeColor('#7e57c2cc', '#011627'));
  assert.equal(colors.selection, colors.accent);
});

test('unique theme names count upwards without clashing', () => {
  assert.equal(uniqueThemeName('Signal', ['Brass']), 'Signal');
  assert.equal(uniqueThemeName('Signal', ['signal', 'Signal 2']), 'Signal 3');
});

test('appearance settings clamp, default and drop unknown themes', () => {
  const parsed = parseAppearance({ scheme: 'purple', themes: { dark: 'missing', light: 'brass' }, glass: 12, radius: 99.6, motion: 'fast',
    fonts: { interface: "Bahnschrift'; }", display: 7 }, backdrop: { kind: 'image', imageId: 'abc', dim: -5, blur: 12.4 },
    custom: [{ id: 'mine', name: '  Mine ', variants: { dark: BUILT_IN_THEMES[0].variants.dark } }, { id: 'signal', name: 'Clash', variants: {} }, 'junk'] });
  assert.equal(parsed.scheme, DEFAULT_APPEARANCE.scheme);
  assert.deepEqual(parsed.themes, { dark: 'carbon', light: 'brass' });
  assert.equal(parsed.glass, 40);
  assert.equal(parsed.radius, 18);
  assert.equal(parsed.motion, 'system');
  assert.deepEqual(parsed.fonts, { interface: 'Bahnschrift', display: '' });
  assert.deepEqual(parsed.backdrop, { kind: 'image', imageId: 'abc', dim: 0, blur: 12 });
  assert.deepEqual(parsed.custom.map(theme => [theme.id, theme.name, Object.keys(theme.variants)]), [['mine', 'Mine', ['dark']]]);
  assert.deepEqual(parseAppearance(null), DEFAULT_APPEARANCE);
  assert.equal(parseAppearance({ custom: parsed.custom, themes: { dark: 'mine' } }).themes.dark, 'mine');
});

test('the first appearance format migrates presets to the default and keeps customised palettes', () => {
  assert.deepEqual(migrateLegacy({ id: 'paper', mode: 'light', colors: {} }), { ...DEFAULT_APPEARANCE, scheme: 'light' });
  const custom = migrateLegacy({ id: 'custom', mode: 'dark', colors: { background: '#101418', surface: '#161c22', raised: '#1e2630', border: '#2c3844',
    text: '#e6edf3', muted: '#9fb0c0', accent: '#ff8800', selection: '#ff8800', positive: '#00aa55', warning: '#ddaa00', error: '#ee4455' } })!;
  assert.equal(custom.themes.dark, 'migrated-custom');
  assert.equal(custom.custom[0].variants.dark!.canvas, '#101418');
  assert.equal(custom.custom[0].variants.dark!.accent, '#ff8800');
  assert.equal(migrateLegacy({ mode: 'sepia', colors: {} }), null);

  const storage = memoryStorage({ 'hoi4-replay.appearance.v1': JSON.stringify({ id: 'atlas', mode: 'dark', colors: {} }) });
  assert.deepEqual(loadAppearance(storage), { settings: DEFAULT_APPEARANCE, warning: null });
});

test('appearance persists and resolves the system scheme', () => {
  const storage = memoryStorage();
  const settings = { ...DEFAULT_APPEARANCE, scheme: 'system' as const, themes: { dark: 'iris', light: 'field' } };
  assert.equal(saveAppearance(settings, storage), null);
  assert.deepEqual(loadAppearance(storage).settings, settings);
  assert.equal(resolveAppearance(settings, true, false).theme.id, 'iris');
  const light = resolveAppearance(settings, false, true);
  assert.equal(light.mode, 'light');
  assert.equal(light.theme.id, 'field');
  assert.equal(light.reducedMotion, true);
  assert.equal(loadAppearance(memoryStorage({ [APPEARANCE_KEY]: '{' })).warning?.includes('could not be restored'), true);
  assert.match(saveAppearance(settings, { getItem: () => null, setItem: () => { throw new Error('full'); } })!, /could not save/);
});

test('HSV conversion round-trips for the colour picker', async () => {
  const { hsvToRgb, rgbToHsv } = await import('./color.ts');
  for (const value of ['#000000', '#ffffff', '#ff0000', '#00ff7f', '#5cb2ff', '#8f82ff', '#7a7a7a']) {
    assert.equal(toHex(hsvToRgb(rgbToHsv(parseColor(value)!))), value);
  }
  assert.deepEqual(rgbToHsv({ r: 0, g: 0, b: 255 }), { h: 240, s: 1, v: 1 });
});
