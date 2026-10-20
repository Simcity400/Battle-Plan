import test from 'node:test';
import assert from 'node:assert/strict';
import { holderBands, holderSourceBands, reservesAt, sourceParts, validateOccupationReserve } from './occupationReserveData.ts';

const reply = validateOccupationReserve({
  window: { firstHour: 0, lastHour: 100, known: [{ firstHour: 0, lastHour: 60 }], reads: [[0, 23, 0], [24, 60, 24]], series: [
    { name: 'GER@7|0|amount', runs: [[0, 60, '1000', true]] },
    { name: 'GER@7|0|source', runs: [[0, 60, '1', true]] },
    { name: 'GER@7|1|amount', runs: [[0, 23, '500', true], [24, 60, '700', false]] },
    { name: 'GER@7|1|source', runs: [[0, 60, '1', true]] },
    { name: 'GER@7|2|amount', runs: [[24, 60, '-300', true]] },
    { name: 'GER@7|2|source', runs: [[24, 60, '2', true]] },
    { name: 'GER@7|3|amount', runs: [[24, 60, '0', true]] },
    { name: 'GER@7|3|source', runs: [[24, 60, '9', true]] },
  ] },
  holders: [{ tag: 'GER', slot: 7 }],
  sources: { 1: 'POL@20', 2: 'FRA@9' },
  warnings: [],
});

test('repeated sources stay separate entries and negatives never subtract from the bar', () => {
  const [holder] = reservesAt(reply, 30)!.holders;
  assert.deepEqual(holder.entries.map(entry => [entry.ordinal, entry.source, entry.amount]), [[0, 'POL@20', 1000n], [1, 'POL@20', 700n], [2, 'FRA@9', -300n], [3, null, 0n]]);
  assert.equal(holder.positive, 1700n);
  assert.deepEqual(holder.negative.map(entry => entry.amount), [-300n]);
  assert.deepEqual(sourceParts(holder), [{ source: 'POL@20', amount: 1700n }]);
});

test('unrecorded hours are unknown', () => {
  assert.equal(reservesAt(reply, 80), null);
});

test('a holder\'s bands hold each source between its changes', () => {
  const [poland] = holderBands(reply, 'GER@7', ['POL@20']);
  assert.deepEqual(poland.map(run => [run.firstHour, run.lastHour, run.value]), [[0, 23, 1500], [24, 60, 1700]]);
});

test('the holder drill keeps unresolved positive amounts in their own band, by dated segment', () => {
  // Entry 4's source resolves only for hours 0-4; its amount at 5-10 is unresolved there.
  const partial = validateOccupationReserve({ window: { firstHour: 0, lastHour: 10, known: [{ firstHour: 0, lastHour: 10 }], reads: [[0, 10, 0]], series: [
    { name: 'GER@7|4|amount', runs: [[0, 10, '1000', true]] },
    { name: 'GER@7|4|source', runs: [[0, 4, '1', true]] },
  ] }, holders: [{ tag: 'GER', slot: 7 }], sources: { 1: 'POL@20' }, warnings: [] });
  const drill = holderSourceBands(partial, 'GER@7');
  assert.deepEqual(drill.sources, ['POL@20', null]);
  assert.deepEqual(drill.bands.map(band => band.map(run => [run.firstHour, run.lastHour, run.value])), [[[0, 4, 1000]], [[5, 10, 1000]]]);
  // A holder whose sources all resolve has no unresolved band.
  assert.deepEqual(holderSourceBands(reply, 'ITA@8').sources, ['POL@20', 'FRA@9']);
  // FRA's only entry is negative, so its band is empty rather than drawn.
  assert.deepEqual(holderBands(reply, 'GER@7', ['POL@20', 'FRA@9', null])[1], []);
});
