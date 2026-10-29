import test from 'node:test';
import assert from 'node:assert/strict';
import { keyedDays } from '../../dashboard/keyedDays.ts';
import { regionDay, regionSeries, shareText, supremacyRows, validateSupremacy } from './navalSupremacyData.ts';

const reply = validateSupremacy({
  window: { firstHour: 0, lastHour: 47, known: [{ firstHour: 0, lastHour: 47 }], reads: [[0, 23, 23], [24, 47, 47]], series: [
    { name: '5|control', runs: [[0, 23, '1', true], [24, 47, '-1', false]] },
    { name: '5|friendly', runs: [[0, 47, '300000', true]] },
    { name: '5|shareNum', runs: [[0, 47, '300000', true]] },
    { name: '5|shareDen', runs: [[0, 47, '400000', true]] },
    { name: '5|share', runs: [[0, 47, '75000', true]] },
    { name: '6|friendly', runs: [[0, 47, '100000', true]] },
  ] },
  dayHours: 24, regions: [{ id: '5', name: 'North Sea' }, { id: '6', name: 'Baltic' }], scale: 100000, warnings: [],
});
const colours = { friendly: 'f', enemy: 'e', neither: 'n' } as const;
const labels = { friendly: 'Friendly control', enemy: 'Enemy control', neither: 'Contested' } as const;

test('control follows the recorded outcome and a refused control is hatched while raw values stay', () => {
  const rows = supremacyRows(reply, reply.regions, keyedDays(reply.window, [0, 47], 24), colours, labels);
  assert.deepEqual(rows[0].cells.map(cell => cell.parts[0]?.id), ['friendly', 'enemy']);
  assert.equal(rows[1].cells[0].unknown, true);
  const baltic = regionDay(regionSeries(reply.window).get('6'), 10);
  assert.equal(baltic.control, null);
  assert.equal(baltic.friendly, 100000n);
});

test('a read day with no values for a region is unknown, not an empty sea', () => {
  // Every day of region 7 was refused, so it has no series at all; the days are still read.
  const regions = [...reply.regions, { id: '7', name: 'Irish Sea' }];
  const rows = supremacyRows(reply, regions, keyedDays(reply.window, [0, 47], 24), colours, labels);
  assert.deepEqual(rows[2].cells.map(cell => cell.unknown), [true, true]);
  // An unread day stays the recorded gap.
  const unread = supremacyRows(reply, regions, [{ firstHour: 48, lastHour: 71, at: 71, readHour: null }], colours, labels);
  assert.equal(unread[2].cells[0].unknown, undefined);
});

test('the share uses only its own numerator and denominator', () => {
  const day = regionDay(regionSeries(reply.window).get('5'), 10);
  // The share is the recorded native quotient, never recomputed from its parts.
  assert.equal(shareText(day, 100000), '75%');
  assert.equal(shareText({ ...day, shareDen: 0n, share: 0n }, 100000), '0%');
  assert.equal(shareText({ ...day, share: null }, 100000), 'Not recorded');
});
