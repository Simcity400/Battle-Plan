import assert from 'node:assert/strict';
import test from 'node:test';
import { fixed5At, validateFixed5Runs, type NativeFixed5Run } from './fixed5Runs.ts';

test('fixed5 deltas preserve fractions, signed wrapping and equal-read prefix', () => {
  const runs: NativeFixed5Run[] = [{ firstHour: 1, lastHour: 5, raw: '9223372036854775807',
    deltas: '2:1 1:100000', deltaScale: '1', readEachHour: true }];
  assert.equal(validateFixed5Runs(runs), true);
  assert.equal(fixed5At(runs, 2)?.raw, '9223372036854775807');
  assert.equal(fixed5At(runs, 3)?.raw, '-9223372036854775808');
  assert.equal(fixed5At(runs, 4)?.raw, '-9223372036854675808');
  assert.equal(fixed5At(runs, 5)?.raw, '-9223372036854675808');
  assert.equal(fixed5At(runs, 0), null);
  assert.equal(fixed5At(runs, 6), null);
});

test('fixed5 missing, gaps and content evaluation stay distinct', () => {
  const runs: NativeFixed5Run[] = [
    { firstHour: 1, lastHour: 2, raw: '0', readEachHour: true },
    { firstHour: 3, lastHour: 4, missing: 0, readEachHour: true },
    { firstHour: 6, lastHour: 7, raw: '100001', deltas: '1:-1', deltaScale: '1', readEachHour: true },
    { firstHour: 8, lastHour: 9, raw: '100000', readEachHour: false },
  ];
  assert.equal(validateFixed5Runs(runs, 1), true);
  assert.equal(fixed5At(runs, 1)?.raw, '0');
  assert.deepEqual(fixed5At(runs, 3), { raw: null, missing: 0, readEachHour: true });
  assert.equal(fixed5At(runs, 5), null);
  assert.equal(fixed5At(runs, 7)?.raw, '100000');
  assert.equal(fixed5At(runs, 8)?.readEachHour, false);
});

test('fixed5 dynamic scale keeps fractional mod scores compact and exact', () => {
  const runs: NativeFixed5Run[] = [{ firstHour: 1, lastHour: 4, raw: '30014',
    deltas: '1:1 1:-1 1:r2', deltaScale: '15007', readEachHour: true }];
  assert.equal(validateFixed5Runs(runs), true);
  assert.equal(fixed5At(runs, 2)?.raw, '45021');
  assert.equal(fixed5At(runs, 3)?.raw, '30014');
  assert.equal(fixed5At(runs, 4)?.raw, '30016');
});

test('fixed5 wire validation refuses malformed deltas, reasons and runs', () => {
  const run = { firstHour: 1, lastHour: 2, raw: '0', readEachHour: true };
  for (const bad of [{ ...run, raw: '9223372036854775808' }, { ...run, deltas: '1 1' },
    { ...run, deltas: 'r' }, { ...run, deltas: ' 1' }, { ...run, missing: 0 },
    { ...run, deltas: '1:1', deltaScale: '0' }, { ...run, deltaScale: '1' },
    { firstHour: 1, lastHour: 2, missing: 2, readEachHour: true }]) {
    assert.equal(validateFixed5Runs([bad], 1), false);
  }
  assert.equal(validateFixed5Runs([run, run]), false);
});

test('sparse changes hold exact values through long equal rereads and arbitrary raw changes', () => {
  const runs: NativeFixed5Run[] = [{ firstHour: 0, lastHour: 43919, raw: '0',
    deltas: '24:1 6:r17 43889:-1', readEachHour: true }];
  assert.equal(validateFixed5Runs(runs), true);
  for (const [hour, raw] of [[0, '0'], [23, '0'], [24, '100000'], [29, '100000'],
    [30, '100017'], [20000, '100017'], [43918, '100017'], [43919, '17']] as const) {
    assert.equal(fixed5At(runs, hour)?.raw, raw);
  }
});

test('sparse pairs refuse ambiguous counts, zero changes and changes outside the observed span', () => {
  const run = { firstHour: 1, lastHour: 10, raw: '0', readEachHour: true };
  for (const deltas of ['1', '1:1:', '0:1', '01:1', '1:0', '1:r0', '1:r100000',
    '10:1', '6:1 4:1', '1:1  1:1', '1:1 ', '1:9223372036854775807']) {
    assert.equal(validateFixed5Runs([{ ...run, deltas }]), false, deltas);
  }
  assert.equal(validateFixed5Runs([{ ...run, deltas: '9:1' }]), true);
});

test('first sparse change canonically establishes its scale, including the minimum signed delta', () => {
  const run = { firstHour: 1, lastHour: 2, raw: '0', readEachHour: true };
  for (const extra of [{ deltas: '1:2' }, { deltas: '1:-2', deltaScale: '7' },
    { deltas: '1:r1' }, { deltas: '1:r-1' }, { deltas: '1:r-9223372036854775808', deltaScale: '7' },
    { deltas: '1:-9223372036854775808', deltaScale: '2' }]) {
    assert.equal(validateFixed5Runs([{ ...run, ...extra }]), false);
  }
  for (const token of ['1', '-1', '-9223372036854775808']) {
    const runs = [{ ...run, deltas: `1:${token}`, deltaScale: '1' }];
    assert.equal(validateFixed5Runs(runs), true);
    assert.equal(fixed5At(runs, 2)?.raw, token);
  }
});
