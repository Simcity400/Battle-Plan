import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_TYPOGRAPHY, parseTypography, typographyVariables } from './typography.ts';
import { DEFAULT_APPEARANCE, parseAppearance, loadAppearance, saveAppearance } from './settings.ts';

const fonts = { interface: 'Inter Variable', display: 'Source Serif 4 Variable', tables: 'Cascadia Code Variable', diagnostics: 'Consolas' };
test('typography defaults and role-specific ranges survive unreadable preferences', () => {
  assert.deepEqual(parseTypography(null), { advanced: false, sizes: { interface: 16, display: 14, diagnostics: 12 }, smoothing: true, wordWrap: true });
  assert.deepEqual(parseTypography({ sizes: { interface: 99, display: 1, tables: 18.9, diagnostics: -9 } }).sizes, { interface: 20, display: 12, diagnostics: 8 }, 'a saved table size is dropped');
  assert.deepEqual(parseTypography({ sizes: { interface: NaN, display: Infinity, tables: '11' } }).sizes, DEFAULT_TYPOGRAPHY.sizes);
});
test('older two-family preferences keep their fonts and reveal a saved headings override', () => {
  const old = parseAppearance({ version: 2, fonts: { interface: 'Arial', display: 'Georgia' } });
  assert.deepEqual(old.fonts, { interface: 'Arial', display: 'Georgia', tables: '', diagnostics: '' });
  assert.equal(old.typography.advanced, true);
  assert.equal(parseAppearance({ fonts: { interface: 'Arial' } }).typography.advanced, false);
  assert.equal(parseAppearance({ fonts, typography: { advanced: false } }).typography.advanced, false);
});
test('Simple preserves advanced overrides and switching back restores them', () => {
  const settings = parseAppearance({ ...DEFAULT_APPEARANCE, fonts: { ...fonts, interface: 'Missing, Arial, sans-serif' }, typography: { advanced: false, sizes: { interface: 17, display: 19, tables: 15, diagnostics: 8 }, smoothing: false, wordWrap: false } });
  let saved: string | null = null;
  const storage = { getItem: () => saved, setItem: (_key: string, value: string) => { saved = value; } };
  assert.equal(saveAppearance(settings, storage), null);
  assert.deepEqual(loadAppearance(storage).settings, settings);
  const restored = loadAppearance(storage).settings;
  assert.equal(restored.typography.advanced, false);
  assert.equal(typographyVariables(restored.fonts, restored.typography)['--font-size-diagnostics'], '12px', 'Simple diagnostics keep their own default size');
  const advanced = { ...restored, typography: { ...restored.typography, advanced: true } };
  assert.equal(saveAppearance(advanced, storage), null);
  assert.deepEqual(loadAppearance(storage).settings, advanced);
  const variables = typographyVariables(advanced.fonts, advanced.typography);
  assert.equal(variables['--font-size-display'], '19px');
  assert.equal(variables['--font-size-diagnostics'], '8px');
  assert.match(variables['--font-diagnostics'], /^'Consolas'/);
  assert.match(typographyVariables(settings.fonts, settings.typography)['--font-ui'], /^'Missing', 'Arial', sans-serif,/);
});
