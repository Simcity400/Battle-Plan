import test from 'node:test';
import assert from 'node:assert/strict';
import { censusComplete, costRead, countText, derivedIcText, finiteFixed, fixedText, hullShare, nativeIcText, refitShare, tokenName, validateYards, yardsAt } from './navalYards.ts';
import { shipLabel, shipName } from '../widgets/refits/shipNames.ts';

const reply = validateYards({
  window: { firstHour: 0, lastHour: 10, known: [{ firstHour: 0, lastHour: 10 }], reads: [[0, 10, 0]], series: [
    { name: 'GER@7|100|present', runs: [[0, 10, '1', true]] },
    { name: 'GER@7|100|name', runs: [[0, 10, '0', true]] },
    { name: 'GER@7|100|queue|1', runs: [[0, 10, '900', true]] },
    { name: 'GER@7|100|queue|0', runs: [[0, 10, '901', true]] },
    { name: 'GER@7|100|queue|2', runs: [[0, 10, '900', true]] },
    { name: 'GER@7|100|ship|900|hull', runs: [[0, 10, '50000', true]] },
    { name: 'GER@7|100|ship|900|maximum', runs: [[0, 10, '200000', true]] },
    { name: 'GER@7|100|ship|900|designName', runs: [[0, 10, '1', true]] },
    { name: 'GER@7|total|usedDockyards', runs: [[0, 10, '3', true]] },
    { name: 'GER@7|known|NavalRepairQueue', runs: [[0, 10, '1', true]] },
  ] },
  ports: [{ country: 'GER@7', id: '100', name: 'Kiel', ships: [{ id: '900', name: 'Ship' }] }],
  tokens: { 0: { id: 'kiel', name: 'Kiel' }, 1: { id: 'design', name: 'Synthetic cruiser' } }, warnings: [],
}, 'ports');

test('queues keep recorded order and repeats; ships fall back to their design name', () => {
  const at = yardsAt(reply, 'GER', 5)!;
  const port = at.groups.get('100')!;
  assert.deepEqual(port.queue.map(entry => entry.ship), ['901', '900', '900']);
  assert.equal(tokenName(reply, port.measures.get('name'), 'none'), 'Kiel');
  assert.equal(shipName(reply, port.ships.get('900')), 'Synthetic cruiser');
  assert.equal(shipName(reply, undefined), 'Ship name unavailable');
  assert.equal(shipLabel(reply, port.ships.get('900')).source, 'design');
  const named = { id: '900', measures: new Map([['name', 0n], ['designName', 1n]]) };
  assert.deepEqual(shipLabel(reply, named), { name: 'Kiel', source: 'recorded' });
  assert.deepEqual(shipLabel(reply, { id: '900', measures: new Map([['nameRebuilt', 1n]]) }), { name: 'Synthetic cruiser', source: 'rebuilt' });
  assert.equal(hullShare(port.ships.get('900')), 0.25);
  assert.equal(at.totals.get('usedDockyards'), 3n);
  assert.equal(fixedText(150000n), '1.5');
  assert.equal(yardsAt(reply, 'GER', 11), null);
});

test('refit progress is accumulated IC over its cost, and dockyards are unscaled counts', () => {
  // Backend scales: progress and cost are fixed5 IC; dockyards and totals are plain integers.
  const refits = validateYards({
    window: { firstHour: 0, lastHour: 10, known: [{ firstHour: 0, lastHour: 10 }], reads: [[0, 10, 0]], series: [
      { name: 'GER@7|5|live', runs: [[0, 10, '1', true]] },
      { name: 'GER@7|5|progress', runs: [[0, 10, '250000', true]] },
      { name: 'GER@7|5|cost', runs: [[0, 4, '500000', true], [5, 10, '0', false]] },
      { name: 'GER@7|5|dockyards', runs: [[0, 10, '4', true]] },
      { name: 'GER@7|total|dockyards', runs: [[0, 10, '4', true]] },
      { name: 'GER@7|total|censusComplete', runs: [[0, 4, '1', true], [5, 10, '0', false]] },
      { name: 'GER@7|read|ProductionLines', runs: [[0, 10, '0', true]] },
    ] },
    lines: [{ country: 'GER@7', id: '5', name: 'Refit', ships: [] }], tokens: {}, warnings: [],
  }, 'lines');
  const early = yardsAt(refits, 'GER', 2)!, late = yardsAt(refits, 'GER', 6)!;
  assert.equal(refitShare(early, '5'), 0.5);
  // A cost that is not positive leaves the share unavailable rather than inventing one.
  assert.equal(refitShare(late, '5'), null);
  assert.equal(countText(early.groups.get('5')!.measures.get('dockyards')), '4');
  assert.equal(countText(early.totals.get('dockyards')), '4');
  assert.equal(fixedText(early.groups.get('5')!.measures.get('progress')), '2.5');
  // Emptiness is asserted only while the census behind it is complete.
  assert.equal(censusComplete(early, 'censusComplete'), true);
  assert.equal(censusComplete(late, 'censusComplete'), false);
});

test('the refit ratio refuses sentinels and a cost without its supplying read', () => {
  const at = (progress: string, cost: string, dated: boolean) => yardsAt(validateYards({
    window: { firstHour: 0, lastHour: 0, known: [{ firstHour: 0, lastHour: 0 }], reads: [[0, 0, 0]], series: [
      { name: 'GER@7|5|live', runs: [[0, 0, '1', true]] },
      { name: 'GER@7|5|progress', runs: [[0, 0, progress, true]] },
      { name: 'GER@7|5|cost', runs: [[0, 0, cost, true]] },
      ...dated ? [{ name: 'GER@7|read|ProductionLines', runs: [[0, 0, '0', true]] as [number, number, string, boolean][] }] : [],
    ] },
    lines: [{ country: 'GER@7', id: '5', name: 'Refit', ships: [] }], tokens: {}, warnings: [],
  }, 'lines'), 'GER', 0)!;
  assert.equal(refitShare(at('250000', '500000', true), '5'), 0.5);
  // An undated cost (its daily supplier interrupted) gives no ratio; the raw IC stays.
  const undated = at('250000', '500000', false);
  assert.equal(refitShare(undated, '5'), null);
  assert.equal(costRead(undated), undefined);
  assert.equal(fixedText(undated.groups.get('5')!.measures.get('progress')), '2.5');
  for (const sentinel of ['9223372036854775807', '-9223372036854775808', '-9223372036854775807']) {
    assert.equal(finiteFixed(BigInt(sentinel)), false);
    assert.equal(refitShare(at(sentinel, '500000', true), '5'), null, `progress ${sentinel}`);
    assert.equal(refitShare(at('250000', sentinel, true), '5'), null, `cost ${sentinel}`);
  }
});

test('sentinel checks apply to native IC words, never to the derived remaining cost', () => {
  // Cost i64::MAX - 1 with progress -1 leaves a real remainder equal to i64::MAX.
  const remainder = 9223372036854775806n - -1n;
  assert.equal(remainder, 9223372036854775807n);
  assert.equal(derivedIcText(remainder), `${fixedText(remainder)} IC`);
  assert.notEqual(derivedIcText(remainder), 'Not recorded');
  assert.equal(derivedIcText(-9223372036854775808n), `${fixedText(-9223372036854775808n)} IC`);
  assert.equal(derivedIcText(undefined), 'Not recorded');
  // The same word as a native operand is the engine's sentinel.
  assert.equal(nativeIcText(9223372036854775807n), 'Not recorded');
  assert.equal(nativeIcText(9223372036854775806n), `${fixedText(9223372036854775806n)} IC`);
  assert.equal(nativeIcText(-1n), `${fixedText(-1n)} IC`);
});
