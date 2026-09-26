import test from 'node:test';
import assert from 'node:assert/strict';
import { contrast, parseColor, rgbToOklch } from '../shell/appearance/color.ts';
import { BUILT_IN_THEMES, themeColors } from '../shell/appearance/themes.ts';
import { CATEGORY_LIMIT, categoricalPalette, distance, GREY_CHROMA, MARK_CONTRAST, nudgeApart } from './palette.ts';
import { tokensFromValues } from './tokens.ts';

test('the categorical palette reads on every built-in theme surface, in both modes', () => {
  for (const theme of BUILT_IN_THEMES) {
    for (const mode of ['dark', 'light'] as const) {
      const surface = themeColors(theme, mode).surface;
      const palette = categoricalPalette(surface);
      assert.equal(palette.length, CATEGORY_LIMIT);
      for (const color of palette) assert.ok(contrast(parseColor(color)!, parseColor(surface)!) >= MARK_CONTRAST - 0.05, `${theme.id}/${mode} ${color} on ${surface}`);
    }
  }
});

test('nudged colours are readable, never grey, apart from each other and stable', () => {
  const background = parseColor('#16181d')!;
  const input = ['#3a6ea5', '#3b6fa6', '#808080', '#101010'].map(hex => parseColor(hex)!);
  const first = nudgeApart(input, background);
  assert.deepEqual(nudgeApart(input, background), first);
  // Later colours never move earlier ones.
  assert.deepEqual(nudgeApart(input.slice(0, 2), background), first.slice(0, 2));
  for (const color of first) {
    assert.ok(contrast(color, background) >= MARK_CONTRAST - 0.05);
    assert.ok(rgbToOklch(color).c >= GREY_CHROMA * 0.9);
  }
  assert.ok(distance(first[0], first[1]) >= 12);
});

test('chart tokens come from theme variables, with the palette solved for the surface', () => {
  const values: Record<string, string> = { '--surface': '#101214', '--text': '#eeeeee' };
  const tokens = tokensFromValues(variable => values[variable] ?? '#777777', 'Inter');
  assert.equal(tokens.surface, '#101214');
  assert.equal(tokens.grid, '#777777');
  assert.equal(tokens.font, 'Inter');
  assert.deepEqual(tokens.categories, categoricalPalette('#101214'));
});
