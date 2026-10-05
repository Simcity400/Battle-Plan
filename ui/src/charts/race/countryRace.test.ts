import assert from 'node:assert/strict';
import test from 'node:test';
import { countryRaceItems, heldStackSegment, partColours } from './countryRace.ts';
import { stepRace } from './race.ts';

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
