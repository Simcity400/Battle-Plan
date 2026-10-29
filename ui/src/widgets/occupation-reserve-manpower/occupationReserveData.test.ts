import test from 'node:test';
import assert from 'node:assert/strict';
import { holderBands, holderKeyAt, holderSourceBands, reservesAt, sourceParts, sourceWitnesses, validateOccupationReserve } from './occupationReserveData.ts';

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

test('each source is looked up where its own entries prove it held its tag, so a later holder of the tag lends nothing', () => {
  // A second POL holder (slot 21) sends reserve after the first stops; FRA throughout from hour 30.
  const reused = validateOccupationReserve({ window: { firstHour: 0, lastHour: 90, known: [{ firstHour: 0, lastHour: 90 }], reads: [[0, 90, 0]], series: [
    { name: 'GER@7|holder', runs: [[0, 90, '1', true]] },
    { name: 'GER@7|0|amount', runs: [[0, 40, '100', true]] }, { name: 'GER@7|0|source', runs: [[0, 40, '1', true]] },
    { name: 'GER@7|1|amount', runs: [[50, 90, '200', true]] }, { name: 'GER@7|1|source', runs: [[50, 90, '2', true]] },
    { name: 'GER@7|2|amount', runs: [[30, 90, '300', true]] }, { name: 'GER@7|2|source', runs: [[30, 90, '3', true]] },
    { name: 'ITA@8|0|source', runs: [[0, 90, '1', true]] },
  ] }, holders: [{ tag: 'GER', slot: 7 }], sources: { 1: 'POL@20', 2: 'POL@21', 3: 'FRA@9' }, warnings: [] });
  const at = (hour: number, sample: number) => Object.fromEntries(sourceWitnesses(reused, 'GER@7', hour, sample).map(witness => [witness.key, [witness.tag, witness.hour]]));
  assert.deepEqual(at(80, 72), { 'POL@20': ['POL', 40], 'POL@21': ['POL', 72], 'FRA@9': ['FRA', 72] }, 'the shown hour\'s look while named then, else as last named');
  assert.deepEqual(at(20, 0), { 'POL@20': ['POL', 0], 'POL@21': ['POL', 50], 'FRA@9': ['FRA', 30] }, 'a source not named yet is looked up as first named');
  assert.deepEqual(at(35, 0)['FRA@9'], ['FRA', 30], 'named at the hour shown but not at the look hour: its first hour of that naming');
});

/** GER holds POL's reserve from hour 10; both are admitted holders through hour 30. */
const admitted = validateOccupationReserve({ window: { firstHour: 0, lastHour: 40, known: [{ firstHour: 0, lastHour: 30 }], reads: [[0, 30, 0]], series: [
  { name: 'GER@7|holder', runs: [[0, 30, '1', true]] },
  { name: 'ITA@8|holder', runs: [[0, 30, '1', true]] },
  { name: 'GER@7|0|amount', runs: [[10, 30, '400', true]] },
  { name: 'GER@7|0|source', runs: [[10, 30, '1', true]] },
] }, holders: [{ tag: 'GER', slot: 7 }, { tag: 'ITA', slot: 8 }], sources: { 1: 'POL@20' }, warnings: [] });

test('an admitted holder without entries holds no reserve, and only a known hour says so', () => {
  const italy = reservesAt(admitted, 20)!.holders.find(holder => holder.holder === 'ITA@8')!;
  assert.deepEqual([italy.admitted, italy.entries, italy.positive], [true, [], 0n]);
  assert.equal(reservesAt(admitted, 35), null);
  // Without the admission the same reply proves nothing about a holder missing from it.
  assert.equal(reservesAt(reply, 30)!.holders.find(holder => holder.holder === 'ITA@8'), undefined);
  assert.equal(reservesAt(reply, 30)!.holders[0].admitted, false);
});

test('an admitted holder\'s bands are zero before its first entry, never a gap, with no unresolved band', () => {
  const drill = holderSourceBands(admitted, 'GER@7');
  assert.deepEqual(drill.sources, ['POL@20']);
  assert.deepEqual(drill.bands[0].map(run => [run.firstHour, run.lastHour, run.value]), [[0, 9, 0], [10, 30, 400]]);
  assert.deepEqual(holderSourceBands(admitted, 'ITA@8'), { sources: ['POL@20'], bands: [[{ firstHour: 0, lastHour: 30, value: 0, breakBefore: true }]] });
});

test('a renamed holder keeps the history it had under its earlier tag', () => {
  // Slot 4 is OLD through hour 9 and NEW from 10; OLD@4's entries stop where NEW@4's begin.
  const renamed = validateOccupationReserve({ window: { firstHour: 0, lastHour: 30, known: [{ firstHour: 0, lastHour: 30 }], reads: [[0, 30, 0]], series: [
    { name: 'OLD@4|holder', runs: [[0, 9, '1', true]] }, { name: 'NEW@4|holder', runs: [[10, 30, '1', true]] },
    { name: 'OLD@4|0|amount', runs: [[0, 9, '100', true]] }, { name: 'OLD@4|0|source', runs: [[0, 9, '1', true]] },
    { name: 'NEW@4|0|amount', runs: [[10, 30, '250', true]] }, { name: 'NEW@4|0|source', runs: [[10, 30, '1', true]] },
  ] }, holders: [{ tag: 'OLD', slot: 4 }, { tag: 'NEW', slot: 4 }], sources: { 1: 'POL@20' }, warnings: [] });
  const aliases = [{ key: 'OLD@4', firstHour: 0, lastHour: 9 }, { key: 'NEW@4', firstHour: 10, lastHour: 30 }];
  assert.equal(holderKeyAt(aliases, 5), 'OLD@4');
  assert.equal(holderKeyAt(aliases, 15), 'NEW@4');
  const drill = holderSourceBands(renamed, aliases);
  assert.deepEqual(drill.sources, ['POL@20']);
  assert.deepEqual(drill.bands[0].map(run => [run.firstHour, run.lastHour, run.value]), [[0, 9, 100], [10, 30, 250]]);
  // One tag's key alone loses the other alias's hours.
  assert.deepEqual(holderSourceBands(renamed, 'NEW@4').bands[0].map(run => [run.firstHour, run.lastHour]), [[10, 30]]);
  // POL is named where either alias's entries prove it, each only within its own hours.
  assert.deepEqual(sourceWitnesses(renamed, aliases, 5, 5).map(witness => [witness.key, witness.hour]), [['POL@20', 5]]);
});
