import test from 'node:test';
import assert from 'node:assert/strict';
import { HEX_HINT, colorInput, contrast, mixRgb, normalizeColor, oklchToRgb, parseColor, parseColorAlpha, rgbToOklch, toHex } from './color.ts';
import { BUILT_IN_THEMES, colorsMode, contrastColors, contrastWarnings, cssVariable, customRoles, deriveColors, reseed, themeRoles } from './themes.ts';
import type { ThemeColors } from './themes.ts';
import { MAX_THEME_NAME, parseJsonc, parseThemeText, serializeTheme, uniqueThemeName } from './themeFile.ts';
import { latestIconImage } from './iconImage.ts';
import { iconStyleOf, withIconStyle } from './iconStyle.ts';
import type { AppearanceSettings } from './settings.ts';
import { APPEARANCE_KEY, DEFAULT_APPEARANCE, loadAppearance, parseAppearance, parseCustomTheme, resolveAppearance, saveAppearance } from './settings.ts';

const hex = /^#[0-9a-f]{6}$/;
function assertMutedOnFills(colors: ThemeColors, mode: 'dark' | 'light', name: string) {
  const muted = parseColor(colors.muted)!, text = parseColor(colors.text)!;
  for (const base of ['canvas', 'chrome', 'surface', 'overlay'] as const) {
    const hovered = mixRgb(parseColor(colors[base])!, text, mode === 'light' ? 0.05 : 0.06);
    assert.ok(contrast(muted, hovered) >= 4.5, `${name}: secondary text on a hovered ${base} row`);
  }
  assert.ok(contrast(muted, parseColor(colors.raised)!) >= 4.5, `${name}: secondary text on a control`);
}
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
  assert.equal(colorInput(' 346BF1 '), '#346bf1', 'a typed hex colour may leave out the #');
  assert.equal(colorInput('#abc'), '#aabbcc');
  for (const typo of ['#346bf', '346bfz', 'blue', '', '#'])
    assert.equal(colorInput(typo), null, `“${typo}” is refused, so the field keeps it and explains the hex form`);
  assert.match(HEX_HINT, /^Use a hex colour like #[0-9a-f]{6}\.$/);
});

test('OKLCH conversion round-trips sRGB colours', () => {
  for (const value of ['#000000', '#ffffff', '#5cb2ff', '#c2410c', '#0a0f16', '#aac77e', '#ff00ff']) {
    assert.equal(toHex(oklchToRgb(rgbToOklch(parseColor(value)!))), value);
  }
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
    assertMutedOnFills(colors, colorsMode(colors), seeds.canvas);
    assert.ok(contrast(parseColor(colors.accentText)!, parseColor(colors.accent)!) >= 3);
  }
});

test('a base colour left as it was keeps every colour, and a new one keeps colours set by hand', () => {
  const derived = deriveColors({ canvas: '#0a0f16', accent: '#5cb2ff', glow: '#2fc6c0' });
  const edited = { ...derived, surface: '#202020', error: '#ff0000' };
  assert.equal(reseed(edited, 'canvas', '#0a0f16', []), edited, 'an unchanged base colour changes nothing');
  assert.equal(reseed(edited, 'canvas', '#0A0F16', []), edited, 'the same colour in other letters changes nothing');
  assert.equal(reseed(edited, 'accent', 'rgb(92, 178, 255)', []), edited);
  const moved = reseed(edited, 'canvas', '#1a1020', ['surface', 'error']);
  assert.equal(moved.canvas, '#1a1020');
  assert.equal(moved.surface, '#202020');
  assert.equal(moved.error, '#ff0000');
  assert.equal(moved.chrome, deriveColors({ canvas: '#1a1020', accent: '#5cb2ff', glow: '#2fc6c0' }).chrome);
  assert.deepEqual(customRoles(derived), []);
  assert.deepEqual(customRoles(edited).sort(), ['error', 'surface']);
});

test('every built-in theme variant passes the editor contrast checks', () => {
  for (const theme of BUILT_IN_THEMES) {
    for (const mode of ['dark', 'light'] as const) {
      const colors = theme.variants[mode]!;
      assert.deepEqual(contrastWarnings(colors), [], `${theme.name} ${mode}`);
      assertMutedOnFills(colors, mode, `${theme.name} ${mode}`);
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
  assert.throws(() => parseThemeText(JSON.stringify({ format: 'other', canvas: '#000' })), /isn’t a BattlePlan theme/);
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
  assert.deepEqual(parseJsonc('{"a": "x, }", "b": "y,\\"]",}'), { a: 'x, }', b: 'y,"]' });
  assert.deepEqual(parseJsonc('[1, // one\n /* two */ ]'), [1]);
  assert.deepEqual(parseJsonc('[1, /* x */ 2]'), [1, 2]);
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
  for (const theme of BUILT_IN_THEMES) for (const [mode, colors] of Object.entries(theme.variants)) for (let percent = 50; percent < 100; percent += 5) {
    const faded = contrastColors(colors, percent);
    for (const role of ['text', 'muted'] as const) for (const fill of ['canvas', 'chrome', 'surface', 'raised', 'overlay'] as const) {
      const ratio = contrast(parseColor(faded[role])!, parseColor(colors[fill])!);
      assert.ok(ratio >= 4.5, `${theme.name} ${mode} at ${percent}%: ${role} on ${fill} is ${ratio.toFixed(2)}:1`);
    }
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


test('old-format appearance keys and payloads are ignored without crashing or overwriting saved data', () => {
  const legacyPayload = JSON.stringify({ id: 'custom', mode: 'light', colors: { background: '#101418', surface: '#161c22', accent: '#ff8800' } });
  const readKeys: string[] = [];
  const writtenKeys: string[] = [];
  const legacyOnly = memoryStorage({ 'battleplan.appearance.v1': legacyPayload });
  const tracked = {
    getItem: (key: string) => { readKeys.push(key); return legacyOnly.getItem(key); },
    setItem: (key: string, value: string) => { writtenKeys.push(key); legacyOnly.setItem(key, value); },
  };
  assert.deepEqual(loadAppearance(tracked), { settings: DEFAULT_APPEARANCE, warning: null });
  assert.deepEqual(readKeys, [APPEARANCE_KEY]);
  assert.deepEqual(writtenKeys, []);
  assert.equal(legacyOnly.getItem('battleplan.appearance.v1'), legacyPayload);

  const oldUnderCurrentKey = memoryStorage({ [APPEARANCE_KEY]: legacyPayload });
  assert.deepEqual(loadAppearance(oldUnderCurrentKey), { settings: DEFAULT_APPEARANCE, warning: null });
  assert.equal(oldUnderCurrentKey.getItem(APPEARANCE_KEY), legacyPayload);

  const currentSettings = { ...DEFAULT_APPEARANCE, scheme: 'light' as const, themes: { dark: 'iris', light: 'field' } };
  const both = memoryStorage({ 'battleplan.appearance.v1': legacyPayload, [APPEARANCE_KEY]: JSON.stringify(currentSettings) });
  assert.deepEqual(loadAppearance(both), { settings: currentSettings, warning: null });
  assert.equal(both.getItem('battleplan.appearance.v1'), legacyPayload);
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
  assert.equal(loadAppearance(memoryStorage({ [APPEARANCE_KEY]: '{' })).warning?.includes('couldn’t be restored'), true);
  assert.match(saveAppearance(settings, { getItem: () => null, setItem: () => { throw new Error('full'); } })!, /couldn’t save/);
});

test('HSV conversion round-trips for the colour picker', async () => {
  const { hsvToRgb, rgbToHsv } = await import('./color.ts');
  for (const value of ['#000000', '#ffffff', '#ff0000', '#00ff7f', '#5cb2ff', '#8f82ff', '#7a7a7a']) {
    assert.equal(toHex(hsvToRgb(rgbToHsv(parseColor(value)!))), value);
  }
  assert.deepEqual(rgbToHsv({ r: 0, g: 0, b: 255 }), { h: 240, s: 1, v: 1 });
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
  function decodes() {
    const pending: ((value: ReturnType<typeof image> | null) => void)[] = [];
    return { rasterise: () => new Promise<ReturnType<typeof image> | null>(resolve => { pending.push(resolve); }), finish: (i: number, value: ReturnType<typeof image> | null) => pending[i](value) };
  }
  let choices = new Map<string, number>(), d = decodes();
  const first = latestIconImage('map', file, choices, d.rasterise), second = latestIconImage('map', file, choices, d.rasterise);
  d.finish(0, image(1)); d.finish(1, image(2));
  assert.equal(await first, null);
  assert.deepEqual(await second, { image: image(2) });
  choices = new Map(); d = decodes();
  const early = latestIconImage('map', file, choices, d.rasterise), late = latestIconImage('map', file, choices, d.rasterise);
  d.finish(1, image(2)); d.finish(0, image(1));
  assert.deepEqual(await late, { image: image(2) });
  assert.equal(await early, null);
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

test('Save theme keeps the editor and its draft when this device can’t store the theme, and saves it on the next try', async () => {
  const { createServer } = await import('vite');
  type Tree = { props?: Record<string, any> };
  const nodes = (value: any): Tree[] => Array.isArray(value) ? value.flatMap(nodes)
    : value && typeof value === 'object' && 'props' in value ? [value, ...nodes(value.props.children)] : [];
  const hooks = { useState: (initial: unknown) => [typeof initial === 'function' ? initial() : initial, () => {}], useRef: (current: unknown) => ({ current }),
    useCallback: (callback: unknown) => callback, useEffect() {}, useLayoutEffect() {}, useMemo: (make: () => unknown) => make() };
  const host = globalThis as typeof globalThis & { __settingsHooks?: typeof hooks; requestAnimationFrame?: unknown };
  host.__settingsHooks = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'settings-save-fixture', enforce: 'pre', transform(source, id) {
      const name = id.replaceAll('\\', '/');
      if (name.endsWith('/shell/Settings.tsx')) return source.replace(/import \{[^\n]+\} from 'react';/, 'const { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } = globalThis.__settingsHooks;');
      if (name.endsWith('/shell/appearance/ThemeEditor.tsx')) return `export default function ThemeEditor() { return null; }
        export const draftChanged = () => false; export const discardDraft = () => ({}); export const useConfirmPopover = () => ({ ask() {}, element: null });`;
    } }] });
  const frame = host.requestAnimationFrame;
  host.requestAnimationFrame = () => 0;
  try {
    const { default: Settings } = await vite.ssrLoadModule('/ui/src/shell/Settings.tsx') as typeof import('../Settings');
    let stored = false, settings: AppearanceSettings = structuredClone(DEFAULT_APPEARANCE);
    const changes: unknown[] = [];
    const appearance = { settings, resolved: resolveAppearance(settings, true, false), setPreview() {},
      update(change: (previous: typeof settings) => typeof settings) { settings = change(settings); },
      commit(change: (previous: typeof settings) => typeof settings) {
        if (!stored) return 'Appearance is applied for this session. This device couldn’t save it.';
        settings = change(settings); return null;
      } };
    const draft = { id: null, name: 'Dusk', mode: 'dark' as const, variants: { dark: deriveColors({ canvas: '#14121b', accent: '#f2a65a' }) } };
    const editing = { draft, original: draft, session: 1 };
    const editor = () => nodes(Settings({ section: 'appearance', target: null, onTargetShown() {}, folders: null, folderError: null, onFoldersChange() {}, native: null, metadata: null,
      branch: null, library: {} as never, client: {} as never, appearance: appearance as never, themeEditing: editing, onThemeEditing: (change: unknown) => { changes.push(change); } } as never))
      .find(node => typeof node.props?.onSave === 'function')!.props!;
    const failed = editor().onSave(draft);
    assert.deepEqual(failed, { text: 'Couldn’t save the theme on this device.', retry: true }, 'the editor is told the write failed, and that trying again can help');
    assert.deepEqual(changes, [], 'the editor stays open with its draft, so closing BattlePlan still asks about it');
    assert.equal(settings.custom.length, 0, 'a theme the device didn’t keep isn’t listed as saved');
    stored = true;
    assert.equal(editor().onSave(draft), null);
    assert.deepEqual(changes, [null], 'a stored theme closes the editor');
    assert.deepEqual(settings.custom.map(theme => theme.name), ['Dusk']);
    assert.equal(settings.themes.dark, settings.custom[0].id);
  } finally { host.requestAnimationFrame = frame; delete host.__settingsHooks; await vite.close(); }
});

test('an imported theme is announced only once this device stores it with its selection, and the import stays open to try again', async () => {
  const { createServer } = await import('vite');
  type Tree = { props?: Record<string, any> };
  const nodes = (value: any): Tree[] => Array.isArray(value) ? value.flatMap(nodes)
    : value && typeof value === 'object' && 'props' in value ? [value, ...nodes(value.props.children), ...nodes(value.props.actions)] : [];
  const announced: unknown[] = [];
  const hooks = { useState: (initial: unknown) => [typeof initial === 'function' ? initial() : initial, (next: unknown) => { announced.push(next); }],
    useRef: (current: unknown) => ({ current }), useId: () => 'id', useEffect() {}, useLayoutEffect() {} };
  const host = globalThis as typeof globalThis & { __appearanceHooks?: typeof hooks };
  host.__appearanceHooks = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'appearance-import-fixture', enforce: 'pre', transform(source, id) {
      if (id.replaceAll('\\', '/').endsWith('/shell/appearance/AppearanceSettings.tsx'))
        return source.replace(/import \{[^\n]+\} from 'react';/, 'const { useEffect, useId, useLayoutEffect, useRef, useState } = globalThis.__appearanceHooks;');
    } }] });
  try {
    const { default: AppearanceSettings } = await vite.ssrLoadModule('/ui/src/shell/appearance/AppearanceSettings.tsx') as typeof import('./AppearanceSettings');
    let stored = false, settings: AppearanceSettings = structuredClone(DEFAULT_APPEARANCE);
    const appearance = { resolved: resolveAppearance(settings, true, false), notice: null,
      get settings() { return settings; },
      update(change: (previous: AppearanceSettings) => AppearanceSettings) { settings = change(settings); },
      commit(change: (previous: AppearanceSettings) => AppearanceSettings) {
        if (!stored) return 'Appearance is applied for this session. This device couldn’t save it.';
        settings = change(settings); return null;
      } };
    const importer = () => nodes(AppearanceSettings({ appearance: appearance as never, onEdit() {} }))
      .find(node => typeof node.props?.onImport === 'function')!.props!.onImport as (text: string) => unknown;
    const text = JSON.stringify({ name: 'Dusk', appearance: 'dark', canvas: '#14121b', accent: '#f2a65a' });
    assert.deepEqual(importer()(text), { text: 'Couldn’t save the imported theme on this device.', retry: true },
      'the import is told the write failed, so it keeps the pasted theme and offers Try again');
    assert.equal(settings.custom.length, 0, 'a theme the device didn’t keep isn’t listed');
    assert.equal(settings.themes.dark, DEFAULT_APPEARANCE.themes.dark, 'nor is it in use for the session only');
    assert.ok(!announced.some(value => JSON.stringify(value ?? null).includes('Imported')), 'nothing announces the import');
    stored = true;
    assert.equal(importer()(text), null);
    assert.deepEqual(settings.custom.map(theme => theme.name), ['Dusk']);
    assert.equal(settings.themes.dark, settings.custom[0].id, 'the theme and its selection are stored together');
    assert.ok(announced.some(value => JSON.stringify(value ?? null).includes('Imported “Dusk”.')));
  } finally { delete host.__appearanceHooks; await vite.close(); }
});

async function themeEditorFixture(check: (editor: {
  field: (label: string) => Record<string, any>; button: (label: string) => Record<string, any>; saves: unknown[]; closed: () => number; focused: string[];
}) => void) {
  const { createServer } = await import('vite');
  type Tree = { type?: unknown; props?: Record<string, any> };
  const nodes = (value: any): Tree[] => Array.isArray(value) ? value.flatMap(nodes)
    : value && typeof value === 'object' && 'props' in value ? [value, ...nodes(value.props.children)] : [];
  const slots: unknown[] = [];
  let cursor = 0;
  const hooks = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (next: unknown) => { slots[index] = typeof next === 'function' ? next(slots[index]) : next; }];
    },
    useRef: (current: unknown) => { const index = cursor++; return (slots[index] ??= { current }); },
    useEffect() { cursor++; }, useMemo: (make: () => unknown) => { cursor++; return make(); },
  };
  const host = globalThis as typeof globalThis & { __editorHooks?: typeof hooks };
  const page = globalThis as { document?: unknown; HTMLElement?: unknown };
  host.__editorHooks = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'theme-editor-fixture', enforce: 'pre', transform(source, id) {
      const name = id.replaceAll('\\', '/');
      if (name.endsWith('/shell/appearance/ThemeEditor.tsx')) return source.replace(/import \{[^\n]+\} from 'react';/, 'const { useEffect, useMemo, useRef, useState } = globalThis.__editorHooks;');
      if (name.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ openAt() {}, close() {} }); export const Popover = () => null;';
    } }] });
  const focused: string[] = [];
  const frame = host.requestAnimationFrame, element = page.HTMLElement;
  host.requestAnimationFrame = (run: FrameRequestCallback) => { run(0); return 0; };
  page.HTMLElement = class {};
  page.document = { activeElement: null, getElementById: (id: string) => ({ focus: () => focused.push(id) }) };
  try {
    const { default: ThemeEditor } = await vite.ssrLoadModule('/ui/src/shell/appearance/ThemeEditor.tsx') as typeof import('./ThemeEditor');
    const draft = { id: null, name: 'Dusk', mode: 'dark' as const, variants: { dark: deriveColors({ canvas: '#14121b', accent: '#f2a65a' }) } };
    const saves: unknown[] = [];
    let closed = 0;
    const render = () => { cursor = 0; return nodes(ThemeEditor({ draft, original: draft, onDraft() {}, onSave: attempt => { saves.push(attempt); return null; }, onClose() { closed++; }, onPreview() {},
      focusName: false, onFocused() {}, escape: { current: null } })); };
    check({
      field: label => render().find(node => node.props?.label === label)!.props!,
      button: label => render().find(node => node.type === 'button' && ([node.props?.children].flat().includes(label) || node.props?.['aria-label'] === label))!.props!,
      saves, closed: () => closed, focused,
    });
  } finally { host.requestAnimationFrame = frame; page.HTMLElement = element; delete page.document; delete host.__editorHooks; await vite.close(); }
}

test('Save theme with a colour typed in a form it can’t read keeps the editor open and takes the user to that field', () => themeEditorFixture(({ field, button, saves, focused }) => {
  assert.equal(field('Canvas').fieldId, 'theme-colour-canvas');
  field('Canvas').onTyped('#14121');
  assert.equal(field('Canvas').typed, '#14121', 'the field keeps what was typed');
  button('Save theme').onClick();
  assert.deepEqual(saves, [], 'the theme isn’t saved with the colour from before the typing');
  assert.deepEqual(focused, ['theme-colour-canvas'], 'the field that needs correcting takes focus');
  field('Canvas').onTyped(null);
  button('Save theme').onClick();
  assert.equal(saves.length, 1, 'once corrected, the theme saves');
}));

test('a colour still being typed counts as an unsaved change when the editor is closed', () => themeEditorFixture(({ field, button, closed }) => {
  field('Canvas').onPending(true);
  button('Close theme editor').onClick({ currentTarget: null });
  assert.equal(closed(), 0, 'closing asks before the typed colour is lost');
  field('Canvas').onPending(false);
  button('Close theme editor').onClick({ currentTarget: null });
  assert.equal(closed(), 1, 'with nothing typed and nothing changed, the editor closes at once');
}));
