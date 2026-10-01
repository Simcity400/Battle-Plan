import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryStyle } from '../generated/dashboard';
import { categoricalPalette, MARK_CONTRAST } from '../charts/palette.ts';
import { contrast, parseColor } from '../shell/appearance/color.ts';
import { countryLineColours } from './countryLineColours.ts';

const styles = new Map<string, CountryStyle>([
  ['opaque!tag', { tag: 'opaque!tag', name: 'Recorded', color: [19, 41, 73] }],
  ['same colour', { tag: 'same colour', name: null, color: [19, 41, 73] }],
]);
test('country lines keep recorded map colours exactly in raw mode, with palette fallback only when missing', () => {
  const tags = ['opaque!tag', 'same colour', 'missing'];
  const colours = countryLineColours(tags, styles, '#16181b', false);
  assert.equal(colours.get(tags[0]), '#132949');
  assert.equal(colours.get(tags[1]), '#132949');
  assert.equal(colours.get(tags[2]), categoricalPalette('#16181b')[2]);
});
test('fixed mode makes recorded colours readable and distinct without changing raw mode', () => {
  const tags = ['opaque!tag', 'same colour'];
  const colours = countryLineColours(tags, styles, '#16181b', true);
  for (const colour of colours.values()) assert.ok(contrast(parseColor(colour)!, parseColor('#16181b')!) >= MARK_CONTRAST);
  assert.notEqual(colours.get(tags[0]), '#132949');
  assert.notEqual(colours.get(tags[0]), colours.get(tags[1]));
  assert.deepEqual(colours, countryLineColours(tags, styles, '#16181b', true));
});
