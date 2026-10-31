import test from 'node:test';
import assert from 'node:assert/strict';
import { contrast, normalizeColor, oklchToRgb, parseColor, parseColorAlpha, rgbToOklch, toHex } from './color.ts';
import { BUILT_IN_THEMES, contrastColors, contrastWarnings, cssVariable, deriveColors, themeRoles } from './themes.ts';
import { MAX_THEME_NAME, parseJsonc, parseThemeText, serializeTheme, uniqueThemeName } from './themeFile.ts';
import { ICON_IMAGE_SIZE, fitIconImage, latestIconImage } from './iconImage.ts';
import { iconStyleOf, withIconStyle } from './iconStyle.ts';
import { APPEARANCE_KEY, DEFAULT_APPEARANCE, LIMITS, loadAppearance, migrateLegacy, parseAppearance, parseCustomTheme, resolveAppearance, saveAppearance } from './settings.ts';

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
  // A 512 KB theme of many closing brackets parses whole.
  const rules = Array.from({ length: 8000 }, (_, i) => `{"scope": "s${i}", "settings": {"foreground": "#112233",},},`).join('');
  assert.equal((parseJsonc(`{"tokenColors": [${rules}]}`) as { tokenColors: unknown[] }).tokenColors.length, 8000);
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
  // A generated name fits the saved limit, keeps its suffix whole and survives a reload.
  const longest = 'L'.repeat(MAX_THEME_NAME);
  const copy = uniqueThemeName(longest, [longest], ' copy');
  assert.equal(copy, `${'L'.repeat(MAX_THEME_NAME - 5)} copy`);
  assert.equal(uniqueThemeName(longest, [longest, copy], ' copy'), `${'L'.repeat(MAX_THEME_NAME - 7)} copy 2`);
  assert.equal(uniqueThemeName('N'.repeat(45), [], ' copy'), `${'N'.repeat(MAX_THEME_NAME - 5)} copy`);
  assert.equal(uniqueThemeName('Signal', ['Signal'], ' copy'), 'Signal copy');
  assert.equal(uniqueThemeName(`${'S'.repeat(MAX_THEME_NAME)}x`, []), 'S'.repeat(MAX_THEME_NAME));
  assert.equal(uniqueThemeName('  Signal  ', ['Signal']), 'Signal 2');
  const saved = parseCustomTheme({ id: 'custom-copy', name: copy, variants: { dark: Object.fromEntries(themeRoles.map(role => [role, '#123456'])) } });
  assert.equal(saved?.name, copy);
});

test('appearance settings clamp, default and drop unknown themes', () => {
  const parsed = parseAppearance({ scheme: 'purple', themes: { dark: 'missing', light: 'brass' }, glass: 12, radius: 99.6, motion: 'fast', headerFlag: 'huge',
    fonts: { interface: "Bahnschrift'; }", display: 7 }, backdrop: { kind: 'image', imageId: 'abc', dim: -5, blur: 12.4 },
    custom: [{ id: 'mine', name: '  Mine ', variants: { dark: BUILT_IN_THEMES[0].variants.dark } }, { id: 'signal', name: 'Clash', variants: {} }, 'junk'] });
  assert.equal(parsed.scheme, DEFAULT_APPEARANCE.scheme);
  assert.deepEqual(parsed.themes, { dark: 'carbon', light: 'brass' });
  assert.equal(parsed.glass, 40);
  assert.equal(parsed.chromeGlass, 52, 'the sidebar and top bar keep the opacity they had with that glass');
  assert.equal(parsed.radius, 18);
  assert.equal(parsed.motion, 'system');
  assert.equal(parsed.headerFlag, 'native');
  assert.equal(parseAppearance({ headerFlag: 'large' }).headerFlag, 'large');
  assert.deepEqual(parsed.fonts, { interface: 'Bahnschrift', display: '', tables: '', diagnostics: '' });
  assert.deepEqual(parsed.backdrop, { kind: 'image', imageId: 'abc', dim: 0, blur: 12 });
  assert.deepEqual(parsed.custom.map(theme => [theme.id, theme.name, Object.keys(theme.variants)]), [['mine', 'Mine', ['dark']]]);
  assert.deepEqual(parseAppearance(null), DEFAULT_APPEARANCE);
  assert.equal(parseAppearance({ custom: parsed.custom, themes: { dark: 'mine' } }).themes.dark, 'mine');
});

test('contrast only softens text and borders below the unchanged 100% maximum', () => {
  for (const mode of ['dark', 'light'] as const) {
    const colors = BUILT_IN_THEMES[0].variants[mode]!;
    assert.equal(contrastColors(colors, 100), colors);
    assert.equal(contrastColors(colors, 200), colors);
    const faded = contrastColors(colors, 50);
    for (const role of ['text', 'muted'] as const) {
      assert.ok(contrast(parseColor(faded[role])!, parseColor(colors.canvas)!) < contrast(parseColor(colors[role])!, parseColor(colors.canvas)!));
    }
    assert.equal(faded.border, `${colors.border}80`);
    for (const role of ['canvas', 'chrome', 'surface', 'raised', 'overlay', 'accent'] as const) assert.equal(faded[role], colors[role]);
  }
});

test('contrast defaults to 100%, rounds within 50–100%, and clamps older boosted preferences', () => {
  assert.equal(DEFAULT_APPEARANCE.contrast, 100);
  assert.equal(parseAppearance({}).contrast, 100);
  assert.equal(parseAppearance({ contrast: 77 }).contrast, 75);
  assert.equal(parseAppearance({ contrast: 78 }).contrast, 80);
  assert.equal(parseAppearance({ contrast: 7 }).contrast, 50);
  for (const contrast of [100, 105, 150, 200, 999]) assert.equal(parseAppearance({ contrast }).contrast, 100);
  assert.equal(parseAppearance({ contrast: '120' }).contrast, 100);
});

test('sidebar icon styles restore independently and reject unknown ids, icons and colours', () => {
  const image = 'data:image/png;base64,iVBORw0KGgo=';
  const parsed = parseAppearance({ sidebarIcons: {
    analytics: { colour: '#ABC' }, map: { colour: 'rgb(10 20 30)', icon: 'globe' }, settings: { colour: 'invalid' }, folders: { icon: image },
    'page-custom': { colour: '#f80', icon: 'no-such-icon' }, diagnostics: { icon: 'data:text/html;base64,PHA+' }, appearance: '#ffcc00', missing: { colour: '#123456' },
  } });
  assert.deepEqual(parsed.sidebarIcons, { analytics: { colour: '#aabbcc' }, map: { colour: '#0a141e', icon: 'globe' }, folders: { icon: image }, 'page-custom': { colour: '#ff8800' } });
  assert.deepEqual(parseAppearance({}).sidebarIcons, {});
  assert.deepEqual(parseAppearance({ sidebarIcons: [{ colour: '#ff0000' }] }).sidebarIcons, {});
  const storage = memoryStorage();
  assert.equal(saveAppearance(parsed, storage), null);
  assert.deepEqual(loadAppearance(storage).settings.sidebarIcons, parsed.sidebarIcons);
  parsed.sidebarIcons = {};
  saveAppearance(parsed, storage);
  assert.equal(loadAppearance(storage).settings.sidebarIcons.analytics, undefined);
});

test('widget icon styles belong to widget kinds and keep only readable parts', () => {
  const image = 'data:image/png;base64,iVBORw0KGgo=';
  const parsed = parseAppearance({ widgetIcons: {
    'navy-size': { icon: 'anchor', colour: '#3B82F6' }, 'not-installed': { icon: image }, aces: { icon: 'data:image/svg+xml;base64,PHN2Zz4=', colour: 'red' },
    'bad kind': { icon: 'globe' }, constructor: { icon: 'globe' }, factories: { icon: 'javascript:alert(1)' }, research: 'flask',
  } });
  // Kinds are not checked against the installed widgets: a kind's saved data is kept, as layouts keep it.
  assert.deepEqual(parsed.widgetIcons, { 'navy-size': { icon: 'anchor', colour: '#3b82f6' }, 'not-installed': { icon: image } });
  assert.deepEqual(DEFAULT_APPEARANCE.widgetIcons, {});
  assert.deepEqual(parseAppearance({ widgetIcons: null }).widgetIcons, {});
  assert.deepEqual(iconStyleOf<string>(parsed.widgetIcons, 'constructor'), {}, 'a kind never reads the prototype');
  const storage = memoryStorage();
  saveAppearance(parsed, storage);
  assert.deepEqual(loadAppearance(storage).settings.widgetIcons, parsed.widgetIcons);
});

test('changing one part of an icon style keeps the other, and a style with neither part is removed', () => {
  const styles = withIconStyle({}, 'navy-size', { icon: 'anchor' });
  assert.deepEqual(styles, { 'navy-size': { icon: 'anchor' } });
  const coloured = withIconStyle(styles, 'navy-size', { colour: '#ff0000' });
  assert.deepEqual(coloured, { 'navy-size': { icon: 'anchor', colour: '#ff0000' } });
  assert.deepEqual(withIconStyle(coloured, 'navy-size', { icon: null }), { 'navy-size': { colour: '#ff0000' } });
  assert.deepEqual(withIconStyle(withIconStyle(coloured, 'navy-size', { icon: null }), 'navy-size', { colour: null }), {});
  assert.deepEqual(styles, { 'navy-size': { icon: 'anchor' } }, 'the previous styles are left as they were');
});

test('an uploaded picture is drawn whole and centred in the square icon', () => {
  assert.equal(ICON_IMAGE_SIZE, 64);
  assert.deepEqual(fitIconImage(200, 100), { x: 0, y: 16, width: 64, height: 32 });
  assert.deepEqual(fitIconImage(30, 120), { x: 24, y: 0, width: 16, height: 64 });
  assert.deepEqual(fitIconImage(16, 16), { x: 0, y: 0, width: 64, height: 64 });
  assert.deepEqual(fitIconImage(0, 0), { x: 0, y: 0, width: 64, height: 64 }, 'an SVG without its own size fills the square');
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

test('the sidebar and top bar glass is set apart from the main view glass', async () => {
  assert.equal(parseAppearance({ glass: 60, chromeGlass: 95 }).chromeGlass, 95);
  assert.equal(parseAppearance({ glass: 60, chromeGlass: 12 }).chromeGlass, 40);
  assert.equal(parseAppearance({ glass: 78 }).chromeGlass, DEFAULT_APPEARANCE.chromeGlass, 'the default keeps today’s look');
  const { createServer } = await import('vite');
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { applyAppearance } = await vite.ssrLoadModule('/ui/src/shell/appearance/useAppearance.ts') as typeof import('./useAppearance');
    const values = new Map<string, string>();
    const root = { dataset: {}, style: { setProperty: (name: string, value: string) => { values.set(name, value); }, removeProperty() {} } } as unknown as HTMLElement;
    const settings = { ...DEFAULT_APPEARANCE, glass: 60, chromeGlass: 100 };
    const page = globalThis as { document?: unknown };
    page.document = { querySelector: () => null, documentElement: null };
    try { applyAppearance(root, settings, resolveAppearance(settings, false, false)); } finally { delete page.document; }
    assert.equal(values.get('--glass'), '60%');
    assert.equal(values.get('--glass-blur'), '20px');
    assert.equal(values.get('--chrome-opacity'), '100%');
    assert.equal(values.get('--chrome-blur'), '0px', 'a solid sidebar and top bar need no blur');
    // Every glass saved before keeps the sidebar and top bar exactly as its single setting drew them.
    for (let glass = LIMITS.glass[0]; glass <= LIMITS.glass[1]; glass++) {
      const migrated = parseAppearance({ glass, backdrop: { kind: 'image', imageId: 'saved' } });
      page.document = { querySelector: () => null, documentElement: null };
      try { applyAppearance(root, migrated, resolveAppearance(migrated, false, false)); } finally { delete page.document; }
      assert.ok(Math.abs(parseFloat(values.get('--chrome-opacity')!) - (glass * 0.8 + 20)) < 1e-9, `opacity for glass ${glass}`);
      assert.equal(values.get('--chrome-blur'), values.get('--glass-blur'), `blur for glass ${glass}`);
    }
  } finally { await vite.close(); }
});

test('the main view has its own backdrop, or shows the sidebar and top bar’s through it', async () => {
  assert.deepEqual(DEFAULT_APPEARANCE.workspaceBackdrop.kind, 'same', 'nothing changes until a main view backdrop is chosen');
  const own = parseAppearance({ backdrop: { kind: 'image', imageId: 'window' }, workspaceBackdrop: { kind: 'image', imageId: 'main', dim: 20, blur: 99 } });
  assert.deepEqual(own.workspaceBackdrop, { kind: 'image', imageId: 'main', dim: 20, blur: 40 });
  assert.equal(own.backdrop.imageId, 'window', 'each keeps its own image');
  assert.equal(parseAppearance({ workspaceBackdrop: { kind: 'video' } }).workspaceBackdrop.kind, 'same');
  assert.equal(parseAppearance({ backdrop: { kind: 'same' } }).backdrop.kind, DEFAULT_APPEARANCE.backdrop.kind, 'the window always has a backdrop of its own');
  const { createServer } = await import('vite');
  const { createElement } = await import('react');
  const { renderToStaticMarkup } = await import('react-dom/server');
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { applyAppearance } = await vite.ssrLoadModule('/ui/src/shell/appearance/useAppearance.ts') as typeof import('./useAppearance');
    const { default: Backdrop } = await vite.ssrLoadModule('/ui/src/shell/appearance/Backdrop.tsx') as typeof import('./Backdrop');
    const dataset: Record<string, string> = {};
    const root = { dataset, style: { setProperty() {}, removeProperty() {} } } as unknown as HTMLElement;
    const page = globalThis as { document?: unknown };
    page.document = { querySelector: () => null, documentElement: null };
    try {
      for (const [workspace, frosted] of [['same', 'ambient'], ['image', 'image'], ['solid', 'solid']] as const) {
        const settings = { ...DEFAULT_APPEARANCE, backdrop: { ...DEFAULT_APPEARANCE.backdrop, kind: 'ambient' as const },
          workspaceBackdrop: { ...DEFAULT_APPEARANCE.workspaceBackdrop, kind: workspace } };
        applyAppearance(root, settings, resolveAppearance(settings, false, false));
        assert.equal(dataset.backdrop, frosted, 'main view panels frost the backdrop behind them');
      }
    } finally { delete page.document; }
    const layer = renderToStaticMarkup(createElement(Backdrop, { className: 'is-workspace', image: 'data:image/png;base64,AA==',
      backdrop: { kind: 'image', imageId: 'main', dim: 20, blur: 6 } }));
    assert.match(layer, /class="app-backdrop is-workspace" data-kind="image"/);
    assert.match(layer, /--backdrop-dim:20%;--backdrop-blur:6px/, 'each layer carries its own dim and blur');
    assert.match(layer, /app-backdrop-image/);
    assert.doesNotMatch(renderToStaticMarkup(createElement(Backdrop, { image: null, backdrop: { ...DEFAULT_APPEARANCE.backdrop, kind: 'ambient' } })), /app-backdrop-image/);
  } finally { await vite.close(); }
});

test('of overlapping uploads and choices for an icon, only the newest lands, whichever finishes first', async () => {
  const image = (n: number) => `data:image/png;base64,${'A'.repeat(n * 4)}` as const;
  const file = {} as File;
  /** A decode the test finishes by hand, in any order. */
  function decodes() {
    const pending: ((value: ReturnType<typeof image> | null) => void)[] = [];
    return { rasterise: () => new Promise<ReturnType<typeof image> | null>(resolve => { pending.push(resolve); }), finish: (i: number, value: ReturnType<typeof image> | null) => pending[i](value) };
  }
  // The earlier upload finishes first: it is dropped, the later one lands.
  let choices = new Map<string, number>(), d = decodes();
  const first = latestIconImage('map', file, choices, d.rasterise), second = latestIconImage('map', file, choices, d.rasterise);
  d.finish(0, image(1)); d.finish(1, image(2));
  assert.equal(await first, null);
  assert.deepEqual(await second, { image: image(2) });
  // The later upload finishes first: it lands, and the earlier one finishing afterwards is still dropped.
  choices = new Map(); d = decodes();
  const early = latestIconImage('map', file, choices, d.rasterise), late = latestIconImage('map', file, choices, d.rasterise);
  d.finish(1, image(2)); d.finish(0, image(1));
  assert.deepEqual(await late, { image: image(2) });
  assert.equal(await early, null);
  // A glyph or reset chosen meanwhile (the menu counts it in the same map) also wins; another icon's choice does not count.
  choices = new Map(); d = decodes();
  const beaten = latestIconImage('map', file, choices, d.rasterise);
  choices.set('map', (choices.get('map') ?? 0) + 1);
  d.finish(0, image(1));
  assert.equal(await beaten, null);
  d = decodes();
  const kept = latestIconImage('map', file, choices, d.rasterise);
  choices.set('folders', 1);
  d.finish(0, null);
  assert.deepEqual(await kept, { image: null }, 'an unreadable file still reports its refusal when nothing newer came');
});
