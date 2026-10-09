import assert from 'node:assert/strict';
import test from 'node:test';
import { casualtyPairsAt, casualtyPairWindows, pairAmount, validateCasualtyPairs } from './casualtyPairsData.ts';

function fixture() {
  return { branchId: '1', available: true, windows: [{ firstHour: 100, lastHour: 120 }, { firstHour: 121, lastHour: 150 }],
    window: 0, observed: [{ firstHour: 100, lastHour: 105 }, { firstHour: 110, lastHour: 120 }], hasGap: true, warnings: [],
    pairs: [{ identity: '1:10|2:20', firstCountry: 'mod attacker', secondCountry: 'mod victim', runs: [
      { firstHour: 100, lastHour: 105, readHour: 100, hourly: true, firstTaken: '9007199254740993', secondTaken: '-12', incomplete: false },
      { firstHour: 110, lastHour: 110, readHour: 110, hourly: true, firstTaken: '0', secondTaken: '3', incomplete: true },
      { firstHour: 111, lastHour: 120, readHour: 110, hourly: false, firstTaken: '0', secondTaken: '3', incomplete: true },
    ] }] };
}
test('country-pair flows preserve direction, wide signed values and known zero', () => {
  const reply = validateCasualtyPairs(fixture());
  const at = casualtyPairsAt(reply, 105);
  assert.equal(at.flows[0].from, 'mod attacker');
  assert.equal(at.flows[0].to, 'mod victim');
  assert.equal(at.flows[0].value, -12n);
  assert.equal(at.flows[1].value, 9007199254740993n);
  assert.equal(at.flows[0].readHour, 105);
  assert.equal(pairAmount(at.flows[0]), '-12');
  assert.equal(pairAmount(casualtyPairsAt(reply, 112).flows[1]), 'Recorded subtotal · 0');
  assert.equal(casualtyPairsAt(reply, 112).flows[0].readHour, 110);
  assert.equal(at.flows.some(flow => flow.from === flow.to), false);
});
test('country-pair flows never carry across gaps and selections support either or both endpoints', () => {
  const reply = validateCasualtyPairs(fixture());
  assert.deepEqual(casualtyPairsAt(reply, 108).flows, []);
  assert.equal(casualtyPairsAt(reply, 105, tag => tag === 'mod attacker').flows.length, 2);
  assert.deepEqual(casualtyPairsAt(reply, 105, tag => tag === 'mod attacker', true).flows, []);
  assert.deepEqual(casualtyPairsAt(reply, 121).flows, []);
});
test('country-pair windows come from the recording and prefetch only during playback', () => {
  const reply = validateCasualtyPairs(fixture());
  assert.deepEqual(casualtyPairWindows(reply, 110, false), [0]);
  assert.deepEqual(casualtyPairWindows(reply, 110, true), [0, 1]);
  assert.deepEqual(casualtyPairWindows(reply, 150, true), [1]);
  assert.deepEqual(casualtyPairWindows(reply, 99, true), []);
});
test('directed pair IDs follow native holders through renames and refuse overlapping aliases', () => {
  const data = fixture();
  data.pairs[0].runs = [data.pairs[0].runs[0]];
  const alias = { ...data.pairs[0], firstCountry: 'renamed attacker', runs: [
    { ...data.pairs[0].runs[0], firstHour: 110, lastHour: 120, readHour: 110 },
  ] };
  data.pairs.push(alias);
  const reply = validateCasualtyPairs(data);
  const before = casualtyPairsAt(reply, 105).flows[0];
  const after = casualtyPairsAt(reply, 115).flows[0];
  assert.equal(before.id, after.id);
  assert.notEqual(before.from, after.from);
  assert.notEqual(before.id, casualtyPairsAt(validateCasualtyPairs({ ...data, pairs: [{ ...alias, identity: '1:11|2:20' }] }), 115).flows[0].id);
  alias.runs[0].firstHour = 105; alias.runs[0].lastHour = 105; alias.runs[0].readHour = 105;
  assert.throws(() => validateCasualtyPairs(data), /malformed/, 'two simultaneous aliases cannot supply one native pair');
});
test('malformed country-pair replies refuse, including overlapping tag holders', () => {
  const acrossGap = fixture(); acrossGap.pairs[0].runs[0].lastHour = 110;
  assert.throws(() => validateCasualtyPairs(acrossGap));
  const badRead = fixture(); badRead.pairs[0].runs[0].readHour = 101;
  assert.throws(() => validateCasualtyPairs(badRead));
  const carryAcrossGap = fixture(); carryAcrossGap.pairs[0].runs[2].readHour = 105;
  assert.throws(() => validateCasualtyPairs(carryAcrossGap));
  const inexact = fixture(); inexact.pairs[0].runs[0].secondTaken = '1.5';
  assert.throws(() => validateCasualtyPairs(inexact));
  const tooWide = fixture(); tooWide.pairs[0].runs[0].secondTaken = (1n << 127n).toString();
  assert.throws(() => validateCasualtyPairs(tooWide));
  const duplicate = fixture(); duplicate.pairs.push(duplicate.pairs[0]);
  assert.throws(() => validateCasualtyPairs(duplicate));
  const holders = fixture(); holders.pairs.push({ ...holders.pairs[0], identity: '1:11|2:20' });
  assert.throws(() => validateCasualtyPairs(holders));
  const manifest = fixture(); Object.assign(manifest, { window: null });
  assert.throws(() => validateCasualtyPairs(manifest));
});
