import assert from 'node:assert/strict';
import test from 'node:test';
import { barExtent, barParts, barRectangle, keyedEntry, paintOrder, partTotal, raceLayout, raceText, rankItems, rowAt, rowCentre, rowSeen, stepRace, MAX_THICKNESS } from './race.ts';
import type { RaceItem, RaceRow, RaceState } from './race';
import { wrapText } from './draw.ts';

const item = (key: string, value: number, stretch: number | null = 0, hatched = 0): RaceItem =>
  ({ key, value, label: String(value), stretch, segments: [{ id: 'total', value, hatched, color: '#123456' }] });
const width = (state: RaceState, key: string) => state.rows.find(row => row.key === key)!.widths.get('total')!.solid;

test('entries rank highest first with ties by key, up to the count, and no value is never ranked as zero', () => {
  const ranked = rankItems([item('b', 5), item('a', 5), item('c', 9), item('d', Number.NaN), item('e', 1)], 3);
  assert.deepEqual(ranked.map(entry => entry.key), ['c', 'a', 'b']);
  assert.deepEqual(rankItems([item('a', 1)], 0), []);
});






test('when a bar jumps, the scale takes it in at once, so neither the bar nor its value is cut off', () => {
  let state = stepRace(null, [item('a', 10, 1), item('b', 8, 1)], 0, 16, 100);
  assert.equal(state.max, 10 * 1.06);
  state = stepRace(state, [item('a', 100, 2), item('b', 9, 1)], 16, 16, 100);
  assert.equal(width(state, 'a'), 100);
  assert.ok(state.max >= 100 * 1.06, `${state.max}`);
  state = stepRace(state, [item('c', 500, 3), item('a', 100, 2)], 32, 16, 100);
  assert.ok(state.max >= 500 * 1.06, `${state.max}`);
  let gliding = stepRace(null, [item('a', 10, 1)], 0, 16, 100);
  gliding = stepRace(gliding, [item('a', 100, 1)], 16, 16, 100);
  assert.ok(gliding.max < 100 * 1.06);
  assert.ok(width(gliding, 'a') * 1.06 <= gliding.max + 1e-9, 'a gliding bar stays inside the easing scale');
  let down = stepRace(null, [item('a', 100, 1)], 0, 16, 100);
  down = stepRace(down, [item('a', 10, 2)], 16, 16, 100);
  assert.equal(width(down, 'a'), 10);
  assert.ok(down.max > 10 * 1.06);
});




test('split bars stack signed: a negative segment lies left of the baseline, and the parts sum to the total', () => {
  const split: RaceItem = { key: 'a', value: 8, label: '8', stretch: 1, segments: [
    { id: 'mil', value: -2, color: '#111111' }, { id: 'civ', value: 7, hatched: 3, color: '#222222' }] };
  const widths = split.segments.map(segment => ({ solid: segment.value, hatched: segment.hatched ?? 0 }));
  assert.deepEqual(barParts(widths), [
    { segment: 0, low: -2, high: 0, hatched: false },
    { segment: 1, low: 0, high: 7, hatched: false },
    { segment: 1, low: 7, high: 10, hatched: true },
  ]);
  assert.deepEqual(barExtent(widths), { low: -2, high: 10 });
  assert.equal(widths.reduce((sum, width) => sum + partTotal(width), 0), split.value);
  assert.deepEqual(barParts([{ solid: -3, hatched: 1 }]), [{ segment: 0, low: -3, high: 0, hatched: false }, { segment: 0, low: -4, high: -3, hatched: true }]);
  const state = stepRace(null, [split, item('b', 5)], 0, 16, 100);
  assert.equal(state.min, -2 * 1.06);
  assert.equal(state.max, 10 * 1.06);
  assert.equal(stepRace(null, [item('b', 5)], 0, 16, 100).min, 0);
  assert.deepEqual(barExtent([{ solid: -4, hatched: 0 }]), { low: -4, high: 0 });
});

test('a new segment appearing at once is taken in by the axis at once', () => {
  const military = (value: number): RaceItem => ({ key: 'a', value, label: '', stretch: 1, segments: [{ id: 'mil', value, color: '#111111' }] });
  let state = stepRace(null, [military(10)], 0, 16, 100);
  const both: RaceItem = { ...military(10), value: 110, segments: [...military(10).segments, { id: 'civ', value: 100, color: '#222222' }] };
  state = stepRace(state, [both], 16, 16, 100);
  assert.deepEqual(state.rows[0].widths.get('civ'), { solid: 100, hatched: 0 });
  assert.ok(state.max >= 110 * 1.06, `${state.max}`);
  const loss: RaceItem = { ...military(10), value: -40, segments: [{ id: 'mil', value: 10, color: '#111111' }, { id: 'nav', value: -50, color: '#333333' }] };
  state = stepRace(state, [loss], 32, 16, 100);
  assert.ok(state.min <= -50 * 1.06, `${state.min}`);
});

test('hovering finds the bar drawn on top while two rows swap places', () => {
  const layout = raceLayout(400, 330, 2, 100, 40);
  assert.equal(layout.thickness, 20);
  const row = (key: string, centre: number, value: number): RaceRow => ({ key, item: item(key, value), phase: 'staying', since: 0, target: 0,
    y: (centre - layout.top - layout.slot / 2) / layout.slot, alpha: 1, widths: new Map([['total', { solid: value, hatched: 0 }]]) });
  const upper = row('upper', 157, 30);
  const lower = row('lower', 159, 90);
  const hover = (rows: RaceRow[], x: number, y: number) => rowAt(rows, layout, { x, y }, 0, 100)?.key;
  assert.deepEqual(barRectangle(upper, layout, 0, 100), { left: 100, right: 178, top: 147, bottom: 167 });
  assert.deepEqual(paintOrder([upper, lower]).map(entry => entry.key), ['lower', 'upper']);
  assert.deepEqual(paintOrder([lower, upper]).map(entry => entry.key), ['lower', 'upper']);
  assert.equal(hover([upper, lower], 150, 159), 'upper');
  assert.equal(hover([lower, upper], 150, 159), 'upper');
  assert.equal(hover([upper, lower], 300, 158), 'lower');
  assert.equal(hover([lower, upper], 300, 157), 'lower');
  assert.equal(hover([upper, lower], 150, 168), 'lower');
  assert.equal(hover([upper, lower], 40, 157), 'upper');
  assert.equal(hover([upper, lower], 350, 159), 'lower');
  assert.equal(hover([upper, lower], 150, 100), 'upper');
  assert.equal(hover([{ ...upper, phase: 'leaving' }, lower], 150, 157), 'lower');
});


test('a row is seen where it is or where it is gliding to, and not once scrolled away', () => {
  const layout = raceLayout(400, 330, 30, 100, 40, 300);
  assert.equal(rowSeen(layout, { y: 0, target: 0 }), false, 'the first row, reached with Home, has scrolled away');
  assert.equal(rowSeen(layout, { y: 20, target: 20 }), true);
  assert.equal(rowSeen(layout, { y: 2, target: 18 }), true, 'gliding into view');
  assert.equal(rowSeen(layout, { y: 18, target: 2 }), true, 'gliding out of view');
});



test('rows are thin (at most 20 px), hovered by the band of their slot, and reached by the keys in rank order', () => {
  const layout = raceLayout(400, 330, 10, 120, 40);
  assert.equal(layout.left, 120);
  assert.equal(layout.right, 360);
  assert.equal(layout.slot, 30);
  assert.equal(layout.thickness, Math.min(MAX_THICKNESS, 30 * 0.52));
  assert.equal(raceLayout(400, 1000, 2, 120, 40).thickness, MAX_THICKNESS);
  const state = stepRace(null, [item('a', 10), item('b', 5)], 0, 16, 100);
  assert.equal(rowAt(state.rows, layout, { x: 130, y: rowCentre(layout, 1) + 3 }, state.min, state.max)?.key, 'b');
  assert.equal(rowAt(state.rows, layout, { x: 130, y: rowCentre(layout, 5) }, state.min, state.max), null);
  const items = state.items;
  assert.equal(keyedEntry('ArrowDown', null, items), 'a');
  assert.equal(keyedEntry('ArrowDown', 'a', items), 'b');
  assert.equal(keyedEntry('ArrowDown', 'b', items), 'b');
  assert.equal(keyedEntry('ArrowUp', 'b', items), 'a');
  assert.equal(keyedEntry('End', null, items), 'b');
  assert.equal(keyedEntry('Home', 'b', items), 'a');
  assert.equal(keyedEntry('ArrowDown', 'gone', items), 'a');
  assert.equal(keyedEntry('ArrowDown', null, []), null);
  assert.equal(raceText(items, key => key.toUpperCase()), '1. A 10; 2. B 5');
  const native = [{ ...items[0], key: '[7,101]', tag: 'renamed', hour: 20 }];
  assert.equal(raceText(native, (_key, entry) => `${entry?.tag} at ${entry?.hour}`), '1. renamed at 20 10');
});

test('a race with no bar wraps why to the view at its spaces instead of squeezing it onto one line', () => {
  const measure = (text: string) => text.length * 7;
  const why = 'Navies not recorded on 10 Jan 1942: navy totals not available at this hour: task forces: not available';
  const lines = wrapText(why, 280, measure);
  assert.ok(lines.length > 1);
  assert.ok(lines.every(line => measure(line) <= 280), 'every line fits the view');
  assert.equal(lines.join(' '), why, 'no word lost or broken');
  assert.deepEqual(wrapText('Unbreakable', 20, measure), ['Unbreakable'], 'a word wider than the view stands alone');
  assert.deepEqual(wrapText('  ', 100, measure), []);
});
