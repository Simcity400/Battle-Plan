import test from 'node:test';
import assert from 'node:assert/strict';
import { readHourAt, validateStabilityReply } from './stabilityReply.ts';

const run = (firstHour: number, lastHour: number, valueRaw: string) => ({ firstHour, lastHour, valueRaw, startsSegment: false });
const carried = (firstHour: number, lastHour: number, readHour: number) => ({ firstHour, lastHour, readHour });
const reply = (series: unknown[]) => ({ branchId: '1', decimals: 5, warnings: [], series });
const good = { tag: 'GER', stability: [run(0, 9, '96000')], warSupport: [run(0, 9, '75000')], maxCommandPower: [run(0, 4, '8000000'), run(5, 9, '9000000')],
  recorded: true, carried: [carried(1, 4, 0), carried(6, 7, 5), carried(8, 8, 6)], breakdown: null };

test('a stability reply with ordered runs and carried stretches is accepted, including a read that moved back', () => {
  assert.equal(validateStabilityReply(reply([good])).series[0].tag, 'GER');
});

test('overlapping runs or stretches, a carried read not before its hours, or a malformed breakdown are refused', () => {
  assert.throws(() => validateStabilityReply(reply([{ ...good, maxCommandPower: [run(0, 5, '1'), run(5, 9, '2')] }])));
  assert.throws(() => validateStabilityReply(reply([{ ...good, carried: [carried(1, 5, 0), carried(5, 9, 4)] }])));
  assert.throws(() => validateStabilityReply(reply([{ ...good, carried: [carried(1, 4, 1)] }])));
  assert.throws(() => validateStabilityReply(reply([{ ...good, recorded: 'yes' }])));
  assert.throws(() => validateStabilityReply(reply([{ ...good, breakdown: 'x' }])));
  assert.throws(() => validateStabilityReply(reply([{ ...good, stability: [run(0, 1, '1.5')] }])));
  assert.throws(() => validateStabilityReply({ ...reply([good]), decimals: 13 }));
});

test('each shown hour maps to the read its values come from', () => {
  assert.equal(readHourAt(good.carried, 0), 0);
  assert.equal(readHourAt(good.carried, 3), 0);
  assert.equal(readHourAt(good.carried, 5), 5);
  assert.equal(readHourAt(good.carried, 8), 6);
  assert.equal(readHourAt(good.carried, 9), 9);
});
