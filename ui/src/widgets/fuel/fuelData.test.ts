import test from 'node:test';
import assert from 'node:assert/strict';
import type { FuelRead, FuelReply } from '../../generated/dashboard';
import { dayAmounts, formatFuel, fuelRuns, fuelTooltips, fuelView, percentFull, validateFuelReply } from './fuelData.ts';

const Q = 32_768;
const read = (firstHour: number, lastHour: number, fuel: number, capacity: number, day: [number, number] | 'overlaps' | 'notUpdated' | 'ownershipNotRecorded', startsSegment = false): FuelRead => ({
  firstHour, lastHour, startsSegment, fuelRaw: String(fuel), capacityRaw: String(capacity), lendLeaseRaw: '0',
  day: Array.isArray(day) ? { gainedRaw: String(day[0]), burnedRaw: String(day[1]) } : null,
  leftOut: Array.isArray(day) ? null : day,
});
// Day 1: 10 fuel of 20, gained 3.5, burned 1.25. A refresh two hours later overlaps it.
// Day 2 (after a gap): full at 20, gained 2, burned 0. Then a read after a day without states.
const REPLY: FuelReply = {
  branchId: '1', tag: 'GER', quantityScale: Q, gainScale: 100_000, dayHours: 24, warnings: [],
  reads: [
    read(1017, 1018, 10 * Q, 20 * Q, [350_000, 1.25 * Q], true),
    read(1019, 1040, 10.5 * Q, 20 * Q, 'overlaps'),
    { ...read(1065, 1070, 20 * Q, 20 * Q, [200_000, 0], true), lendLeaseRaw: String(1.5 * Q) },
    { ...read(1089, 1089, 7 * Q, 20 * Q, 'notUpdated'), lendLeaseRaw: String(2 * Q) },
    read(1113, 1113, 7 * Q, 20 * Q, 'ownershipNotRecorded'),
  ],
};
const COLORS = { fuel: 'blue', capacity: 'grey', gained: 'azure', burned: 'orange' };
const forms = (hour: number) => {
  const view = fuelView(REPLY, 0, 5000);
  return fuelTooltips(REPLY, view.reads, view.readHours, hour, `Hour ${hour}`, COLORS);
};
const tip = (hour: number) => forms(hour)[0];

test('the reply must have its reads in order, each with its day or the reason it is left out', () => {
  assert.equal(validateFuelReply(REPLY), REPLY);
  const bad = (change: (reply: FuelReply) => unknown) => {
    const copy = structuredClone(REPLY);
    change(copy);
    assert.throws(() => validateFuelReply(copy), /malformed/);
  };
  bad(reply => { reply.reads[1].firstHour = 1018; });
  bad(reply => { reply.reads[0].leftOut = 'overlaps'; });
  bad(reply => { reply.reads[1].leftOut = null; });
  bad(reply => { reply.reads[0].fuelRaw = '1.5'; });
  bad(reply => { (reply.reads[0] as { lendLeaseRaw?: string }).lendLeaseRaw = undefined; });
  bad(reply => { reply.quantityScale = 0; });
  bad(reply => { (reply.reads[1] as { leftOut: string }).leftOut = 'later'; });
  bad(reply => { (reply.reads[1] as { leftOut: string }).leftOut = 'toString'; });
});

test('a date range cuts the reads, keeps every read hour for dating, and keeps only the days read inside it', () => {
  const view = fuelView(REPLY, 1030, 1067);
  assert.deepEqual(view.reads.map(item => [item.firstHour, item.lastHour, item.startsSegment]), [[1030, 1040, false], [1065, 1067, true]]);
  assert.deepEqual(view.readHours, [1017, 1019, 1065, 1089, 1113]);
  // Day 2's bar is cut to the range's start; day 1 was read before it.
  assert.deepEqual(view.bars, [{ firstHour: 1041, lastHour: 1065, up: 2, down: 0 }]);
  assert.deepEqual(fuelView(REPLY, 0, 5000).bars, [
    { firstHour: 993, lastHour: 1017, up: 3.5, down: 1.25 },
    { firstHour: 1041, lastHour: 1065, up: 2, down: 0 },
  ]);
  const runs = fuelRuns(REPLY.reads, Q, 'fuel');
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.value, run.breakBefore]),
    [[1017, 1018, 10, true], [1019, 1040, 10.5, false], [1065, 1070, 20, true], [1089, 1089, 7, false], [1113, 1113, 7, false]]);
  assert.deepEqual(fuelRuns(REPLY.reads, Q, 'capacity').map(run => run.value), [20, 20, 20, 20, 20]);
});

test('amounts are exact: each part and the net are rounded once from the recorded values', () => {
  // Gained 0.005 and burned 1/32768 (0.0000305…): the net rounds from 0.0049694…
  assert.deepEqual(dayAmounts(REPLY, read(1, 1, 0, Q, [500, 1])), { gained: 1n, burned: 0n, net: 0n });
  assert.deepEqual(dayAmounts(REPLY, REPLY.reads[0]), { gained: 350n, burned: 125n, net: 225n });
  assert.equal(dayAmounts(REPLY, REPLY.reads[1]), null);
  assert.equal(formatFuel(String(1_234_567.5 * Q), Q, 0), '1,234,568');
  assert.equal(formatFuel('-16384', Q), '−0.5');
  assert.equal(formatFuel(String(20 * Q), Q), '20');
  assert.equal(percentFull(REPLY.reads[1]), 53n);
  assert.equal(percentFull(read(1, 1, 5, 0, 'overlaps')), null);
});

test('the tooltip lists the tank, then the day before its read, and dates a carried read', () => {
  const day = tip(1018);
  assert.deepEqual(day.rows.map(row => [row.label, row.value]), [
    ['Fuel', '10'], ['Capacity', '20'], ['Full', '50%'], ['Gained (own production)', '+3.5'], ['Burned', '−1.25'], ['Net', '+2.25']]);
  assert.equal(day.rows[5].tone, 'positive');
  assert.match(day.note!, /^Read .*; fuel is read once a day\. Gained and burned in the 24 hours before the read; lend-lease fuel received is not included\.$/);
  // Without lend-lease at the read nothing more is said; with it, its last hour is named, never a day's total.
  assert.doesNotMatch(day.note!, /arriving/);
  assert.match(tip(1065).note!, /Lend-lease fuel was arriving at the read \(\+1\.5 in its last hour\); the tank gains it, but these figures leave it out\./);
  assert.match(tip(1017).note!, /^Gained and burned/);
  // A full tank says why the level did not rise by the net.
  assert.match(tip(1065).note!, /discards fuel beyond its capacity/);
  const overlapping = tip(1019);
  assert.deepEqual(overlapping.rows.at(-1), { label: 'Gained and burned', value: 'Counted in another day', tone: 'muted', divider: true });
  assert.match(overlapping.note!, /counted in that read’s bar/);
  assert.match(tip(1089).note!, /held no state/);
  // A day the game did not update makes no claim from its frozen lend-lease value.
  assert.doesNotMatch(tip(1089).note!, /lend-lease/i);
  // Unknown ownership is its own reason, never a bar.
  assert.equal(tip(1113).rows.at(-1)!.value, 'Not known');
  assert.match(tip(1113).note!, /Part of the day was not recorded/);
  assert.deepEqual(tip(1050).rows, [{ label: 'Not recorded at this time', tone: 'muted' }]);
});

test('the compact tooltip form keeps the tank level and the net', () => {
  assert.deepEqual(forms(1018)[1].rows.map(row => row.label), ['Fuel', 'Net']);
});
