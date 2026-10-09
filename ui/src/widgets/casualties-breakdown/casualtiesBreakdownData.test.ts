import assert from 'node:assert/strict';
import test from 'node:test';
import type { CasualtyBreakdownReply, CasualtyBreakdownSeries } from '../../generated/dashboard';
import { breakdownWindows, lossItems, lossesAt, lossStack, validateBreakdown } from './casualtiesBreakdownData.ts';

const series = (id: string, value: string, enemyIdentity = id): CasualtyBreakdownSeries => ({ id, countryIdentity: '1:10',
  enemyTag: `synthetic enemy ${id}`, enemyIdentity, runs: [{ firstHour: 100, lastHour: 102, readHour: 100, hourly: true,
    groupingHour: 100, groupingHourly: false, enemyHour: 100, enemyHourly: false,
    value, warId: '0', reason: null, incomplete: false }] });
const reply = (): CasualtyBreakdownReply => ({ branchId: '1', tag: 'synthetic country', available: true,
  windows: [{ firstHour: 100, lastHour: 102 }], window: 0, observed: [{ firstHour: 100, lastHour: 102 }],
  countries: [{ firstHour: 100, lastHour: 102, identity: '1:10', incomplete: false }],
  series: [series('positive', '12'), series('correction', '-3'), series('zero', '0')],
  wars: [{ id: '0', firstHour: 100, lastHour: 102, startedBy: true, endedBy: true,
    names: [{ firstHour: 100, lastHour: 100, name: { text: 'First native name', known: true, reason: null, key: null } },
      { firstHour: 101, lastHour: 102, name: { text: 'Later native name', known: true, reason: null, key: null } }] }],
  hasGap: false, warnings: [] });

test('signed corrections and zero rows remain exact while geometry includes only positive amounts', () => {
  const data = validateBreakdown(reply());
  const at = lossesAt(data, 102)!;
  assert.equal(at.total, 9n);
  assert.equal(at.rows.length, 3);
  assert.equal(at.rows[1].value, -3n);
  const items = lossItems(at, at, tag => tag);
  assert.equal(items.reduce((sum, item) => sum + (item.value ?? 0), 0), 12);
  assert.ok(items.every(item => item.value === null || item.value >= 0));
  assert.equal(lossesAt(data, 103), null);
});

test('an ended relation keeps its supplied name and removed enemy facts without future backfill', () => {
  const at = lossesAt(reply(), 102)!;
  assert.equal(at.rows[0].war, 'First native name');
  assert.equal(at.rows[0].groupingHour, 100);
  assert.equal(at.rows[0].enemyHour, 100);
  assert.equal(at.rows[0].readHour, 102);
});

test('overlapping native groups have one exact unidentified path with the reason visible', () => {
  const data = reply();
  data.series[0].runs[0].warId = null;
  data.series[0].runs[0].reason = 'Multiple native episodes';
  const at = lossesAt(validateBreakdown(data), 100)!;
  assert.equal(at.total, 9n);
  assert.equal(at.rows[0].war, 'Other / not identified');
  assert.equal(at.rows[0].reason, 'Multiple native episodes');
});

test('malformed gaps, holder replacement, duplicate lifetimes and future supplying dates refuse', () => {
  const gap = reply(); gap.observed = [{ firstHour: 100, lastHour: 100 }, { firstHour: 102, lastHour: 102 }];
  assert.throws(() => validateBreakdown(gap), /malformed/);
  const replaced = reply(); replaced.countries[0].identity = '1:11';
  assert.throws(() => validateBreakdown(replaced), /malformed/);
  const duplicate = reply(); duplicate.series.push(duplicate.series[0]);
  assert.throws(() => validateBreakdown(duplicate), /malformed/);
  const future = reply(); future.series[0].runs[0].groupingHour = 103;
  assert.throws(() => validateBreakdown(future), /malformed/);
  const overflow = reply(); overflow.series[0].runs[0].value = '9223372036854775808';
  assert.throws(() => validateBreakdown(overflow), /malformed/);
});

test('Fill never inherits a replacement holder and recording blocks define prefetch', () => {
  const now = lossesAt(reply(), 100)!;
  const end = { ...now, identity: '1:11' };
  assert.ok(lossItems(now, end, tag => tag).every(item => item.finalUnknown && item.final === 0));
  assert.deepEqual(breakdownWindows({ ...reply(), windows: [{ firstHour: 100, lastHour: 102 }, { firstHour: 103, lastHour: 105 }] }, 101, true), [0, 1]);
  assert.deepEqual(breakdownWindows(reply(), 103, true), []);
});

test('five thousand synthetic countries remain selectable and exact through palette folding', () => {
  const data = reply(); data.series = Array.from({ length: 5000 }, (_, index) => series(String(index), index === 0 ? '-1' : '1'));
  const at = lossesAt(validateBreakdown(data), 101)!;
  assert.equal(at.rows.length, 5000);
  assert.equal(at.total, 4998n);
  const stack = lossStack([data]);
  assert.equal(stack.model.edges[0]!.at(-1)!.high, 4999);
});

test('an incomplete final snapshot retains positive ordering weights without claiming a Fill whole', () => {
  const now = lossesAt(reply(), 100)!;
  const end = { ...now, incomplete: true };
  const items = lossItems(now, end, tag => tag);
  assert.ok(items.every(item => item.finalUnknown));
  assert.equal(items.reduce((sum, item) => sum + (item.final ?? 0), 0), 12);
  assert.equal(items.reduce((sum, item) => sum + (item.exact ?? 0n), 0n), 12n);
  assert.equal(end.total, 9n);
});
