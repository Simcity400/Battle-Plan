import assert from 'node:assert/strict';
import test from 'node:test';
import { countryRaceEnds, countryRaceHours, countryRaceItems, countryRacePickTag, countryRaceReadHour, countryRaceSegments, countryRaceStack, countryRaceTooltipTags, forEachCountryRaceSegment, forEachJoinedCountryRaceSegment, heldStackSegment, partColours } from './countryRace.ts';
import { barExtent, selectRaceRow, stepRace } from './race.ts';
import { linesAt, linesTooltip } from './lines.ts';
import { buildStack, stackedSegments } from '../stack.ts';
import { lineLabelHours } from '../series.ts';
import { fitTooltip } from '../tooltip.ts';

test('drill selection accepts row clicks, Enter and Space and preserves the selected hour', () => {
  const calls: [string | null, number][] = [];
  const select = (tag: string) => calls.push([tag, 123]);
  for (const trigger of ['click', 'Enter', ' ']) assert.equal(selectRaceRow('opaque tag', trigger, select), true);
  assert.equal(selectRaceRow(null, 'click', select), false);
  assert.equal(selectRaceRow('opaque tag', 'ArrowDown', select), false);
  assert.equal(selectRaceRow('opaque tag', 'Enter'), false);
  assert.deepEqual(calls, [['opaque tag', 123], ['opaque tag', 123], ['opaque tag', 123]]);
  const lines = [{ tag: 'first country', runs: [] }, { tag: 'second country', runs: [] }];
  assert.equal(countryRacePickTag('lines', lines), null);
  assert.equal(countryRacePickTag('lines', lines.slice(0, 1)), 'first country');
  assert.equal(countryRacePickTag('mix', lines, 'mix country'), 'mix country');
  assert.equal(countryRacePickTag('mix', lines), null);
});

test('sampled race looks and Jump use each value read and never invent a missing sample', () => {
  const hours = new Map([['mod a|', 119], ['mod b|', 143]]);
  assert.equal(countryRaceReadHour('mod a|', 150, hours), 119);
  assert.equal(countryRaceReadHour('mod b|', 150, hours), 143);
  assert.equal(countryRaceReadHour('unknown', 150, hours), null);
  assert.equal(countryRaceReadHour('mod a|', 150), 150);
  const readHour = (_tag: string | null, hour: number) => hour < 119 ? null : hour < 143 ? 119 : 143;
  assert.equal(countryRaceReadHour('mod a|', 118, undefined, readHour), null);
  assert.equal(countryRaceReadHour('mod a|', 130, undefined, readHour), 119);
  assert.equal(countryRaceReadHour('mod a|', 150, undefined, readHour), 143);
});

test('country races keep amounts exact while rows move and type colours follow content order', () => {
  const colours = partColours([{ id: 'mod_type', label: 'Mod stockpile' }], ['palette']);
  const entries = (value: number) => countryRaceItems([{ tag: 'AAA', value, label: String(value), stretch: 1,
    parts: [{ id: 'mod_type', value }] }], colours, () => 'country', 'unknown');
  const before = stepRace(null, entries(2), 0, 0, 100);
  const after = stepRace(before, entries(8), 16, 16, 100);
  assert.equal(after.rows[0].widths.get('mod_type')?.solid, 8);
  assert.equal(after.rows[0].item.segments[0].color, 'palette');
});

test('country mix expands shared stack samples without inventing intermediate values', () => {
  assert.deepEqual(heldStackSegment({ hours: [1, 3], bands: [{ low: [0, 0], high: [2, 8] }] }),
    { hours: [1, 3, 3], bands: [{ low: [0, 0, 0], high: [2, 2, 8] }] });
});

test('type colours keep full content order when chosen parts are renamed or folded', () => {
  const parts = [{ id: 'first', label: 'Mod type' }, { id: 'second', label: 'Another type' }, { id: 'last', label: 'Renamed type' }];
  assert.deepEqual([...partColours(parts, ['one', 'two'])], [['first', 'one'], ['second', 'two'], ['last', 'one']]);
  assert.deepEqual([...partColours(parts.map(part => ({ ...part, label: 'Other name' })), ['one', 'two'])], [...partColours(parts, ['one', 'two'])]);
  assert.equal(partColours([{ id: 'other', label: 'Other', color: 'muted' }], ['one']).get('other'), 'muted');
  assert.equal(partColours([parts[1]], ['one', 'two'], partColours(parts, ['one', 'two'])).get('second'), 'two');
});

test('inside hatching is bounded by positive counts, while residual hatching adds to solid parts', () => {
  const [total, split, negative, residual] = countryRaceItems([
    { tag: 'total', value: 10, label: '10', stretch: 1, hatched: 12 },
    { tag: 'split', value: 8, label: '8', stretch: 1, parts: [
      { id: 'negative', value: -2, hatched: 3, hatchInside: true },
      { id: 'positive', value: 10, hatched: 4, hatchInside: true },
    ] },
    { tag: 'negative', value: -2, label: '−2', stretch: 1, hatched: 3 },
    { tag: 'residual', value: 10, label: '10', stretch: 1, parts: [{ id: 'unknown', value: 2, hatched: 8 }] },
  ], new Map([['positive', 'type']]), () => 'country', 'missing');
  assert.deepEqual(total.segments, [{ id: 'total', value: 0, hatched: 10, color: 'country' }]);
  assert.deepEqual(split.segments, [{ id: 'negative', value: -2, hatched: 0, color: 'missing' }, { id: 'positive', value: 6, hatched: 4, color: 'type' }]);
  assert.deepEqual(barExtent(split.segments.map(part => ({ solid: part.value, hatched: part.hatched ?? 0 }))), { low: -2, high: 10 });
  assert.deepEqual(negative.segments, [{ id: 'total', value: -2, hatched: 0, color: 'country' }]);
  assert.equal(residual.segments[0].hatched, 8);
  assert.deepEqual(barExtent(residual.segments.map(part => ({ solid: part.value, hatched: part.hatched ?? 0 }))), { low: 0, high: 10 });
});

test('country-specific colours override the catalogue and unsplit totals keep country colours', () => {
  const entries = countryRaceItems([
    { tag: 'a', value: 4, label: '4', stretch: 1, parts: [{ id: 'same', value: 4, color: 'a-type' }] },
    { tag: 'b', value: 4, label: '4', stretch: 1, parts: [{ id: 'same', value: 4, color: 'b-type' }] },
    { tag: 'c', value: 4, label: '4', stretch: 1 },
  ], new Map([['same', 'catalogue']]), tag => tag, 'missing', false);
  assert.deepEqual(entries.map(entry => entry.segments[0].color), ['a-type', 'b-type', 'c']);
  assert.deepEqual(entries.map(entry => entry.stretch), [1, 1, 1]);
});

test('optional amount easing stays within one recorded stretch and labels remain exact', () => {
  const entries = (value: number, stretch: number) => countryRaceItems([{ tag: 'a', value, label: String(value), stretch }], new Map(), () => 'country', 'missing', false);
  let state = stepRace(null, entries(2, 1), 0, 16, 100);
  state = stepRace(state, entries(8, 1), 16, 16, 100);
  assert.ok(state.rows[0].widths.get('total')!.solid > 2 && state.rows[0].widths.get('total')!.solid < 8);
  assert.equal(state.rows[0].item.label, '8');
  state = stepRace(state, entries(12, 3), 32, 16, 100);
  assert.equal(state.rows[0].widths.get('total')!.solid, 12);
});

test('legacy mixes use the same half-pixel thinning and preserve signed peaks and gaps', () => {
  const stack = buildStack([[{ firstHour: 0, lastHour: 1, value: 2 }, { firstHour: 2, lastHour: 2, value: 12 },
    { firstHour: 3, lastHour: 4, value: -8 }, { firstHour: 8, lastHour: 10, value: 3, breakBefore: true }]]);
  const legacy = countryRaceSegments(stack, 1, 10, 8, false);
  assert.deepEqual(legacy, stackedSegments(stack, 1, 10, 4));
  assert.ok(legacy.some(segment => segment.bands[0].high.includes(12)));
  assert.ok(legacy.some(segment => segment.bands[0].low.includes(-8)));
  assert.equal(legacy.length, 2);
  assert.deepEqual(countryRaceSegments(stack, 1, 10, 8), legacy.map(heldStackSegment));
  assert.deepEqual(countryRaceSegments(stack, 5, 7, 8), []);
});

test('keyboard stops are sorted change hours, without synthetic stack end boundaries', () => {
  assert.deepEqual(countryRaceHours([{ tag: 'a', runs: [{ firstHour: 2, lastHour: 4, value: 2 }, { firstHour: 8, lastHour: 10, value: 3 }] },
    { tag: 'b', runs: [{ firstHour: 1, lastHour: 6, value: 3 }, { firstHour: 8, lastHour: 10, value: 4 }] }]), [1, 2, 8]);
});

test('end labels stop at each drawn read and retain exact raw text', () => {
  const lines = [{ tag: 'a', runs: [{ firstHour: 1, lastHour: 4, value: 2 }, { firstHour: 8, lastHour: 10, value: 3 }] }];
  const name = (_tag: string, hour?: number) => hour === undefined ? 'Moment' : `At ${hour}`;
  const color = (_tag: string, hour?: number) => hour === undefined ? 'moment-colour' : `colour-${hour}`;
  assert.deepEqual(countryRaceEnds(lines, 1, 7, null, name, color, String), [{ id: 'a', color: 'colour-4', value: 2, text: 'At 4', suffix: '2' }]);
  assert.deepEqual(countryRaceEnds(lines, 1, 10, 9, name, color, String).map(end => [end.text, end.suffix]), [['At 9', '3']]);
  assert.deepEqual(countryRaceEnds([...lines, { tag: 'b', runs: [{ firstHour: 1, lastHour: 3, value: 8 }] }], 1, 10, 9, name, color, String),
    [{ id: 'a', color: 'colour-9', value: 3, text: 'At 9', suffix: '3' }, { id: 'b', color: 'colour-3', value: 8, text: 'At 3', suffix: '8' }]);
  assert.deepEqual(countryRaceEnds(lines, 5, 7, null, name, color, String), []);
  assert.deepEqual(countryRaceEnds([{ ...lines[0], color: 'capacity', muted: true, endLabel: hour => `Exact ${hour}: 9,007,199,254,740,993` }],
    1, 7, null, name, color, String), [{ id: 'a', color: 'capacity', value: 2, muted: true, text: 'Exact 4: 9,007,199,254,740,993' }]);
});

test('each line has its own end hour, capped at the playhead and visible range', () => {
  const lines = [{ tag: 'early', runs: [{ firstHour: 100, lastHour: 199, value: 2 }] },
    { tag: 'late', runs: [{ firstHour: 100, lastHour: 300, value: 3 }] }, { tag: 'future', runs: [{ firstHour: 250, lastHour: 300, value: 4 }] }];
  assert.deepEqual([...lineLabelHours(lines, 100, 1000, null)], [['early', 199], ['late', 300], ['future', 300]]);
  assert.deepEqual([...lineLabelHours(lines, 100, 1000, 205)], [['early', 199], ['late', 205]]);
  assert.deepEqual([...lineLabelHours(lines, 200, 240, 205)], [['late', 205]]);
});

test('country secondary measures keep their own endpoints and opaque identities', () => {
  const lines = [{ tag: 'MOD A', id: JSON.stringify(['MOD A', 'primary']), measure: 'primary',
    runs: [{ firstHour: 1, lastHour: 8, value: 10 }] },
  { tag: 'MOD A', id: JSON.stringify(['MOD A', 'secondary']), measure: 'secondary', runs: [{ firstHour: 1, lastHour: 4, value: 3 }] }];
  const ends = countryRaceEnds(lines, 1, 8, null, (_tag, hour) => `Country at ${hour}`, () => 'country-colour', String);
  assert.deepEqual(ends.map(end => [end.id, end.text, end.suffix]), [[lines[0].id, 'Country at 8 · primary', '10'], [lines[1].id, 'Country at 4 · secondary', '3']]);
  assert.deepEqual(linesAt(4, lines), ['MOD A']);
  const entries = countryRaceItems([{ tag: 'MOD A', value: 10, label: '10 / 3', stretch: 1, inset: 3 }], new Map(), () => 'country', 'missing', false);
  assert.deepEqual(entries[0].inset, { value: 3, color: 'country' });
});

test('line tooltips use the hovered hour for country names and flags, omitting missing countries', () => {
  const lines = [{ tag: 'a', runs: [{ firstHour: 1, lastHour: 10, value: 2 }] }, { tag: 'b', runs: [{ firstHour: 1, lastHour: 4, value: 3 }] }];
  const labelAt = (hour: number) => (tag: string) => ({ name: `${tag} at ${hour}`, ...(hour < 5 ? { flag: 'early-flag' } : {}) });
  assert.deepEqual(linesTooltip(4, lines, labelAt, () => 'country', 'Day', 'Count', String).rows.map(row => [row.label, row.countryFlag?.flag]),
    [['b at 4', 'early-flag'], ['a at 4', 'early-flag']]);
  assert.deepEqual(linesAt(8, lines), ['a']);
  assert.deepEqual(linesTooltip(8, lines, labelAt, () => 'country', 'Day', 'Count', String).rows.map(row => row.label), ['a at 8']);
  assert.deepEqual(linesTooltip(12, lines, labelAt, () => 'country', 'Day', 'Count', String).rows.map(row => row.label), ['Not recorded']);
  for (const state of [{ flagPending: true }, { flagReason: 'Content sprite absent', flagPending: false },
    { flagReason: 'Dated request failed', flagPending: false }, { flag: 'decoded-at-8', flagPending: false }]) {
    const label = { name: 'a at 8', ...state };
    const tip = linesTooltip(8, lines, () => () => label, () => 'country', 'Day', 'Count', String);
    assert.deepEqual(tip.rows[0].countryFlag, label, 'native country presence and the same dated read state survive without a URL');
    assert.equal(tip.rows[0].icon, undefined);
  }
});

test('shared fitting keeps supplied totals and reads the full compacted tooltip aloud', () => {
  const full = { title: 'Country', rows: [{ label: 'Level', value: '10' }, { label: 'Ceiling', value: '20' },
    ...Array.from({ length: 10 }, (_, index) => ({ label: `Part ${index}`, value: '1' })), { label: 'Net', value: '2' }] };
  const compact = { ...full, rows: [full.rows[0], full.rows.at(-1)!] };
  const fitted = fitTooltip([full, compact], 120, { keep: 1 });
  assert.deepEqual(fitted.rows.map(row => row.label), ['Level', 'Net']);
  assert.match(fitted.spoken!, /Ceiling 20; Part 0 1/);
  const noTotal = fitTooltip([full], 90, { keep: 0 });
  assert.ok(!noTotal.rows.some(row => row.label === 'Net'));
});

test('one country compares independent measures without stacking or bridging their gaps', () => {
  const lines = [{ tag: 'MOD A', id: 'resistance', runs: [{ firstHour: 1, lastHour: 4, value: 20 }] },
    { tag: 'MOD A', id: 'compliance', runs: [{ firstHour: 1, lastHour: 8, value: 70 }] }];
  assert.deepEqual(countryRaceEnds(lines, 1, 10, null, (_tag, hour) => `Name at ${hour}`, () => 'country', String),
    [{ id: 'resistance', color: 'country', value: 20, text: 'Name at 4', suffix: '20' },
      { id: 'compliance', color: 'country', value: 70, text: 'Name at 8', suffix: '70' }]);
  assert.deepEqual(linesAt(3, lines), ['MOD A']);
  assert.deepEqual(linesAt(6, lines), ['MOD A']);
  assert.deepEqual(linesAt(9, lines), []);
  assert.deepEqual(countryRaceTooltipTags(6, ['MOD A'], (hour, tags) => [`${tags[0]} ${hour}`, 'MOD A', 'MOD']), ['MOD A', 'MOD A 6', 'MOD']);
});

test('related hover countries are deduplicated at the hovered hour for 5000 countries', () => {
  assert.deepEqual(countryRaceTooltipTags(101, ['Mod country', 'Mod country'],
    (hour, tags) => [`${tags[0]} at ${hour}`, 'Other', 'Other']), ['Mod country', 'Mod country at 101', 'Other']);
  const tags = Array.from({ length: 5000 }, (_, index) => `Mod country ${index}`);
  assert.deepEqual(countryRaceTooltipTags(102, tags, () => tags), tags);
  assert.deepEqual(countryRaceTooltipTags(102, []), []);
});


test('a stack changing at every read fills as one shape per stretch, never as seamed strips', () => {
  const days = Array.from({ length: 60 }, (_, day) => day);
  const band = (scale: number) => [...days.map(day => ({ firstHour: day * 24, lastHour: day * 24 + 23, value: scale * (day % 7 + 1) })),
    { firstHour: 70 * 24, lastHour: 71 * 24 + 23, value: scale, breakBefore: true }];
  const stack = countryRaceStack([band(1), band(2)], true);
  const pieces: number[] = [];
  forEachCountryRaceSegment(stack, 0, 72 * 24, 1, true, () => { pieces.push(1); });
  const joined: { hours: number[] }[] = [];
  forEachJoinedCountryRaceSegment(stack, 0, 72 * 24, 1, true, segment => { joined.push(segment); });
  assert.ok(pieces.length > joined.length);
  assert.equal(joined.length, 2);
  for (const segment of joined) assert.ok(segment.hours.every((hour, index) => index === 0 || segment.hours[index - 1] <= hour));
  assert.ok(joined[0].hours.at(-1)! < joined[1].hours[0]);
});

