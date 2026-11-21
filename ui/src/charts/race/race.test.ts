import assert from 'node:assert/strict';
import test from 'node:test';
import { barExtent, barLength, barParts, barRectangle, keyedEntry, MIN_SLOT, paintOrder, partTotal, RACE_AXIS, raceHeight, raceLayout, raceMax, raceMoving, raceText, rankItems, rowAt, rowCentre, rowInView, rowSeen, stepRace, MAX_THICKNESS, RACE_LEAVE_MS } from './race.ts';
import type { RaceItem, RaceRow, RaceState } from './race';
import { drawRaceBar, wrapText } from './draw.ts';
import { tokensFromValues } from '../tokens.ts';

const item = (key: string, value: number, stretch: number | null = 0, hatched = 0): RaceItem =>
  ({ key, value, label: String(value), stretch, segments: [{ id: 'total', value, hatched, color: '#123456' }] });
const order = (state: RaceState) => [...state.rows].filter(row => row.phase !== 'leaving').sort((a, b) => a.target - b.target).map(row => row.key);
const width = (state: RaceState, key: string) => state.rows.find(row => row.key === key)!.widths.get('total')!.solid;

test('entries rank highest first with ties by key, up to the count, and no value is never ranked as zero', () => {
  const ranked = rankItems([item('b', 5), item('a', 5), item('c', 9), item('d', Number.NaN), item('e', 1)], 3);
  assert.deepEqual(ranked.map(entry => entry.key), ['c', 'a', 'b']);
  assert.deepEqual(rankItems([item('a', 1)], 0), []);
});

test('the first state shows every row in place; a new moment re-ranks and rows glide to their new slots', () => {
  const first = [item('a', 10), item('b', 5)];
  let state = stepRace(null, first, 0, 16, 100);
  assert.deepEqual(order(state), ['a', 'b']);
  assert.deepEqual(state.rows.map(row => [row.key, row.y, row.alpha]), [['a', 0, 1], ['b', 1, 1]]);
  assert.equal(raceMoving(state), false);
  const second = [item('b', 20), item('a', 12)];
  state = stepRace(state, second, 16, 16, 100);
  assert.deepEqual(order(state), ['b', 'a']);
  const b = state.rows.find(row => row.key === 'b')!;
  assert.ok(b.y < 1 && b.y > 0, `${b.y}`);
  assert.ok(width(state, 'b') > 5 && width(state, 'b') < 20);
  assert.equal(raceMoving(state), true);
  for (let now = 32; now < 5000; now += 16) state = stepRace(state, second, now, 16, 100);
  assert.equal(raceMoving(state), false);
  assert.deepEqual(state.rows.map(row => [row.key, row.y, width(state, row.key)]), [['b', 0, 20], ['a', 1, 12]]);
});

test('rows out of view land at once and keep their row at rest; a row coming into view glides', () => {
  const view = { from: 0, to: 2 };
  let state = stepRace(null, [item('a', 50), item('b', 40), item('c', 30), item('d', 20), item('e', 10)], 0, 16, 100);
  const row = (key: string) => state.rows.find(row => row.key === key)!;
  const next = [item('a', 60), item('b', 40), item('c', 30), item('e', 25), item('d', 20)];
  state = stepRace(state, next, 16, 16, 100, undefined, view);
  assert.deepEqual([row('d').y, row('e').y, width(state, 'e')], [4, 3, 25], 'swapped below the view: landed');
  assert.ok(width(state, 'a') > 50 && width(state, 'a') < 60, 'in view: glides');
  const held = row('d');
  state = stepRace(state, next, 32, 16, 100, undefined, view);
  assert.equal(row('d'), held, 'a row at rest out of view is the same row');
  state = stepRace(state, [item('e', 99), item('a', 60), item('b', 40), item('c', 30), item('d', 20)], 48, 16, 100, undefined, view);
  assert.ok(row('e').y > 0 && row('e').y < 3, `climbing into view, it glides: ${row('e').y}`);
});

test('a bar jumps across a gap or a change of country, and without motion everything snaps', () => {
  let state = stepRace(null, [item('a', 10, 100)], 0, 16, 100);
  state = stepRace(state, [item('a', 30, 200)], 16, 16, 100);
  assert.equal(width(state, 'a'), 30);
  state = stepRace(state, [item('a', 40, null)], 32, 16, 100);
  assert.equal(width(state, 'a'), 40);
  state = stepRace(state, [item('a', 50, null)], 48, 16, 100);
  assert.equal(width(state, 'a'), 50);
  let still = stepRace(null, [item('a', 10), item('b', 5)], 0, 16, 0);
  still = stepRace(still, [item('b', 20), item('a', 12)], 16, 16, 0);
  assert.deepEqual(still.rows.map(row => [row.key, row.y, width(still, row.key)]), [['b', 0, 20], ['a', 1, 12]]);
});

test('inner measures ease independently without increasing the total and keep overflow in the axis', () => {
  const nested = (total: number, inner: number, stretch = 1) => ({ ...item('a', total, stretch), inset: { value: inner, color: 'country' } });
  let state = stepRace(null, [nested(10, 2)], 0, 16, 100);
  assert.equal(barLength(state.items[0]), 10);
  state = stepRace(state, [nested(20, 8)], 16, 16, 100);
  assert.ok(width(state, 'a') > 10 && width(state, 'a') < 20);
  assert.ok(state.rows[0].inset! > 2 && state.rows[0].inset! < 8);
  assert.equal(raceMoving(state), true);
  state = stepRace(state, [nested(3, 12, 2)], 32, 16, 100);
  assert.equal(width(state, 'a'), 3);
  assert.equal(state.rows[0].inset, 12);
  assert.equal(barLength(state.items[0]), 12);
  assert.ok(state.max >= 12 * 1.06);
  const rectangle = barRectangle(state.rows[0], raceLayout(500, 200, 1, 60, 30), 0, state.max);
  assert.ok(rectangle.right > rectangle.left);
  state = stepRace(state, [nested(3, 12, 2)], 48, 16, 0);
  assert.equal(raceMoving(state), false);
});

test('shared race drawing keeps the inner measure thinner and draws success-only overflow', () => {
  const layout = raceLayout(500, 200, 1, 60, 30);
  const state = stepRace(null, [{ ...item('a', 0), inset: { value: 15, color: 'country' } }], 0, 16, 0);
  const points: number[][] = [];
  const fills: string[] = [];
  const context = { fillStyle: '', strokeStyle: '', lineWidth: 1, globalAlpha: 1,
    save() {}, restore() {}, beginPath() {}, closePath() {}, clip() {}, stroke() {},
    moveTo(x: number, y: number) { points.push([x, y]); }, lineTo(x: number, y: number) { points.push([x, y]); },
    arcTo(x: number, y: number) { points.push([x, y]); }, fillRect() {}, fill() { fills.push(context.fillStyle); },
  };
  const tip = drawRaceBar(context as unknown as CanvasRenderingContext2D, layout, tokensFromValues(() => '#000000', 'sans-serif'), state.rows[0], 0, state.max);
  assert.deepEqual(fills, ['country']);
  assert.equal(tip, Math.max(...points.map(point => point[0])));
  assert.equal(Math.max(...points.map(point => point[1])) - Math.min(...points.map(point => point[1])), layout.thickness / 2);
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

test('a new entry comes in from below at its value, and one that is gone slides away and fades before it is dropped', () => {
  let state = stepRace(null, [item('a', 10), item('b', 5)], 0, 16, 100);
  state = stepRace(state, [item('a', 10), item('c', 8)], 16, 16, 100);
  const c = state.rows.find(row => row.key === 'c')!;
  assert.equal(c.phase, 'entering');
  assert.ok(c.y > 1, `enters from below: ${c.y}`);
  assert.ok(c.alpha < 1);
  assert.equal(width(state, 'c'), 8);
  const b = state.rows.find(row => row.key === 'b')!;
  assert.equal(b.phase, 'leaving');
  assert.ok(b.target > 1);
  assert.equal(b.item.value, 5);
  for (let now = 32; now < 16 + RACE_LEAVE_MS + 32; now += 16) state = stepRace(state, state.items, now, 16, 100);
  assert.deepEqual(state.rows.map(row => row.key), ['a', 'c']);
});

test('a row that is gone eases to a destination fixed when it went, the same at any frame rate, and comes back from nearby', () => {
  const tau = 180;
  const rows = (keys: string) => [...keys].map((key, index) => item(key, 100 - index));
  const leave = (fps: number, after: number) => {
    let state = stepRace(null, rows('abcd'), 0, 16, tau);
    const shown = rows('acd');
    const frames = Math.round(after * fps / 1000);
    for (let frame = 1; frame <= frames; frame++) state = stepRace(state, shown, frame * 1000 / fps, 1000 / fps, tau);
    return state;
  };
  const gone = (state: RaceState) => state.rows.find(row => row.key === 'b')!;
  assert.equal(gone(leave(24, 375)).target, 3);
  assert.equal(gone(leave(48, 375)).target, 3);
  const expected = 3 + (1 - 3) * Math.exp(-375 / tau);
  for (const fps of [24, 40, 48]) assert.ok(Math.abs(gone(leave(fps, 375)).y - expected) < 1e-9, `${fps} fps: ${gone(leave(fps, 375)).y}`);
  let shrink = stepRace(null, rows('ab'), 0, 16, tau);
  shrink = stepRace(shrink, rows('a'), 16, 16, tau);
  assert.equal(shrink.rows.find(row => row.key === 'b')!.target, 2);
  let state = leave(60, 400);
  const was = gone(state).y;
  assert.ok(was <= 3, `${was}`);
  state = stepRace(state, rows('abcd'), 420, 20, tau);
  const back = gone(state);
  assert.equal(back.phase, 'staying');
  assert.equal(back.target, 1);
  assert.ok(back.y < was && back.y > 1, `${back.y}`);
});

test('each segment eases its solid and hatched parts on its own, and a new segment appears at its length', () => {
  const split = (civilian: number, damaged: number, naval?: number): RaceItem => ({ key: 'a', value: civilian + (naval ?? 0), label: '', stretch: 1,
    segments: [{ id: 'civ', value: civilian, hatched: damaged, color: '#111111' }, ...(naval == null ? [] : [{ id: 'nav', value: naval, color: '#222222' }])] });
  let state = stepRace(null, [split(10, 4)], 0, 16, 100);
  assert.deepEqual(state.rows[0].widths.get('civ'), { solid: 10, hatched: 4 });
  state = stepRace(state, [split(12, 0, 6)], 16, 16, 100);
  const civ = state.rows[0].widths.get('civ')!;
  assert.ok(civ.solid > 10 && civ.solid < 12 && civ.hatched > 0 && civ.hatched < 4, JSON.stringify(civ));
  assert.deepEqual(state.rows[0].widths.get('nav'), { solid: 6, hatched: 0 });
  assert.equal(raceMoving(state), true);
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

test('rows keep a height that fits their flag and name; rows that need more room scroll under a fixed axis', () => {
  assert.equal(raceHeight(10), 8 + 10 * MIN_SLOT + RACE_AXIS);
  const fits = raceLayout(400, 330, 10, 100, 40, 50);
  assert.deepEqual([fits.top, fits.bottom, fits.slot], [8, 308, 30]);
  assert.deepEqual(fits.view, { top: 8, bottom: 308 });
  const top = raceLayout(400, 330, 30, 100, 40);
  assert.equal(top.slot, MIN_SLOT);
  assert.equal(raceHeight(30) - 330, 300);
  assert.deepEqual([top.top, top.bottom], [8, 608]);
  const scrolled = raceLayout(400, 330, 30, 100, 40, 120);
  assert.deepEqual(scrolled.view, top.view);
  assert.deepEqual([scrolled.top, scrolled.bottom], [8 - 120, 608 - 120]);
  const end = raceLayout(400, 330, 30, 100, 40, 1000);
  assert.equal(end.bottom, end.view.bottom);
  assert.equal(rowCentre(end, 29), end.view.bottom - MIN_SLOT / 2);
  const state = stepRace(null, Array.from({ length: 30 }, (_, index) => item(`r${index}`, 30 - index)), 0, 16, 100);
  assert.equal(rowInView(top, rowCentre(top, 14)), true);
  assert.equal(rowInView(top, rowCentre(top, 16)), false);
  assert.equal(rowAt(state.rows, top, { x: 50, y: top.view.bottom + 10 }, state.min, state.max), null);
  assert.equal(rowAt(state.rows, scrolled, { x: 50, y: rowCentre(scrolled, 20) }, state.min, state.max)?.key, 'r20');
  assert.equal(raceLayout(400, 40, 5, 100, 40, 1000).bottom, 40 - RACE_AXIS);
  const fewer = raceLayout(400, 330, 30, 100, 40, 1000, 20);
  assert.equal(fewer.slot, MIN_SLOT);
  assert.deepEqual([fewer.top, fewer.bottom], [8 - (raceHeight(20) - 330), fewer.view.bottom]);
  assert.equal(raceLayout(400, 330, 30, 100, 40, 100, 12).top, 8);
});

test('a row is seen where it is or where it is gliding to, and not once scrolled away', () => {
  const layout = raceLayout(400, 330, 30, 100, 40, 300);
  assert.equal(rowSeen(layout, { y: 0, target: 0 }), false, 'the first row, reached with Home, has scrolled away');
  assert.equal(rowSeen(layout, { y: 20, target: 20 }), true);
  assert.equal(rowSeen(layout, { y: 2, target: 18 }), true, 'gliding into view');
  assert.equal(rowSeen(layout, { y: 18, target: 2 }), true, 'gliding out of view');
});

test('a race of 5,000 entries reaches its last row by scrolling', () => {
  const layout = raceLayout(800, 400, 5000, 120, 40, raceHeight(5000));
  assert.equal(layout.slot, MIN_SLOT);
  assert.equal(rowInView(layout, rowCentre(layout, 4999)), true);
  assert.equal(rowInView(layout, rowCentre(layout, 0)), false);
  assert.equal(layout.top, layout.view.bottom - 5000 * MIN_SLOT);
});

test('the axis ends a little past the longest bar, hatched parts included', () => {
  assert.equal(barLength(item('a', 10, 0, 4)), 14);
  assert.equal(raceMax([item('a', 10, 0, 4), item('b', 2)]), 14 * 1.06);
  assert.equal(raceMax([]), 1.06);
  assert.equal(barLength({ ...item('a', -3), segments: [{ id: 'x', value: -3, hatched: -1, color: '' }] }), 3);
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
