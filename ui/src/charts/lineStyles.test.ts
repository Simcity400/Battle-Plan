import assert from 'node:assert/strict';
import test from 'node:test';
import { lineColours, lineDashes, lineStyles } from './palette.ts';

const palette = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'];
const lines = (keys: readonly number[]) => keys.map(key => ({ key, preferred: key - 1 }));
const look = (style: { color: string; primary: readonly number[]; secondary: readonly number[] }) => `${style.color}|${style.primary.join(',')}|${style.secondary.join(',')}`;

test('chosen lines never share a colour, and keep their own where it is free', () => {
  const colours = lineColours(lines([9, 1, 10, 2]), palette);
  assert.deepEqual([...colours].sort((a, b) => a[0] - b[0]), [[1, 'a'], [2, 'b'], [9, 'c'], [10, 'd']]);
  assert.equal(new Set(lineColours(lines([3, 4, 5, 6, 7, 8, 9, 10]), palette).values()).size, 8);
  assert.equal(lineColours(lines([4]), palette).get(4), 'd');
});

test('no two lines look alike however many there are', () => {
  for (const count of [8, 20, 25, 40]) {
    const styles = lineStyles(lines(Array.from({ length: count }, (_, index) => index + 1)), palette);
    assert.equal(new Set([...styles.values()].map(look)).size, count, `${count} lines`);
    for (const style of styles.values()) assert.notDeepEqual(style.primary, style.secondary);
  }
  const styles = lineStyles(lines([1, 9]), palette);
  assert.deepEqual(styles.get(1), { color: 'a', primary: [], secondary: [6, 4] });
  assert.equal(lineStyles(lines([3]), palette).get(3)!.color, lineColours(lines([3]), palette).get(3));
  const patterns = Array.from({ length: 50 }, (_, round) => lineDashes(round));
  assert.equal(new Set(patterns.flatMap(pattern => [pattern.primary.join(','), pattern.secondary.join(',')])).size, 100);
});
