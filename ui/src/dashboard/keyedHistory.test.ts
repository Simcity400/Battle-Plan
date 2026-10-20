import test from 'node:test';
import assert from 'node:assert/strict';
import { countryPart, keyedChanges, keyedKnown, keyedRead, keyedValuesAt, runAtHour, validateKeyedWindow } from './keyedHistory.ts';

const window = validateKeyedWindow({
  firstHour: 0, lastHour: 30,
  known: [{ firstHour: 0, lastHour: 9 }, { firstHour: 20, lastHour: 30 }],
  reads: [[0, 4, 0], [5, 9, 5], [20, 30, 20]],
  series: [
    { name: 'GER@7|1', runs: [[0, 9, '5', true], [20, 25, '-170141183460469231731687303715884105728', true]] },
    { name: 'ITA@8|1', runs: [[5, 9, '0', true]] },
  ],
});

test('values are looked up at the hour; absence inside known hours stays absent', () => {
  assert.deepEqual([...keyedValuesAt(window, 3)], [['GER@7|1', '5']]);
  assert.deepEqual([...keyedValuesAt(window, 6)], [['GER@7|1', '5'], ['ITA@8|1', '0']]);
  assert.deepEqual([...keyedValuesAt(window, 27)], []);
  assert.equal(keyedKnown(window, 27), true);
  assert.equal(runAtHour(window.series[0].runs, 22)?.[2], '-170141183460469231731687303715884105728');
});

test('hours outside the known spans are not recorded and carry no read', () => {
  assert.equal(keyedKnown(window, 12), false);
  assert.deepEqual([...keyedValuesAt(window, 12)], []);
  assert.equal(keyedRead(window, 12), undefined);
  assert.equal(keyedRead(window, 7), 5);
});

test('country parts keep the slot apart from the tag', () => {
  assert.deepEqual(countryPart('D01 mod@19'), { tag: 'D01 mod', slot: 19 });
  assert.equal(countryPart('GER'), null);
});

test('change stops cover starts, ends and known spans', () => {
  assert.deepEqual(keyedChanges(window), [0, 5, 10, 20, 26]);
});

test('malformed windows are refused', () => {
  assert.throws(() => validateKeyedWindow({ ...window, series: [{ name: 'x', runs: [[3, 1, '1', true]] }] }));
  assert.throws(() => validateKeyedWindow({ ...window, series: [{ name: 'x', runs: [[1, 3, '1.5', true]] }] }));
  assert.throws(() => validateKeyedWindow({ ...window, known: [{ firstHour: 5, lastHour: 9 }, { firstHour: 0, lastHour: 2 }] }));
});
