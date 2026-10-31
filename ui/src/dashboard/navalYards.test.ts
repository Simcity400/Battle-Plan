import test from 'node:test';
import assert from 'node:assert/strict';
import { censusComplete, costRead, countText, derivedIcText, finiteFixed, fixedText, hullShare, nativeIcText, refitShare, tokenName, validateYards, yardsAt } from './navalYards.ts';
import type { YardsReply } from './navalYards.ts';
import { shipLabel, shipName } from '../widgets/refits/shipNames.ts';

/** A reply at an hour: its holders and totals, the source tables' dated reads and the tokens' names. */
const replyOf = (holders: YardsReply['holders'], totals: Record<string, string>, reads: Record<string, number> = {},
  known = true): YardsReply => validateYards({
  hour: 5, known, readHour: 5, holders, totals, reads, warnings: [],
  tokens: { 0: { id: 'kiel', name: 'Kiel' }, 1: { id: 'design', name: 'Synthetic cruiser' } },
});

const reply = replyOf([{
  id: '100', measures: { present: '1', name: '0' },
  queue: [{ ordinal: '0', ship: '901' }, { ordinal: '1', ship: '900' }, { ordinal: '2', ship: '900' }],
  ships: [{ id: '900', measures: { hull: '50000', maximum: '200000', designName: '1' } }],
}], { usedDockyards: '3' });

test('queues keep recorded order and repeats; ships fall back to their design name', () => {
  const at = yardsAt(reply)!;
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
  // An hour whose countries were not recorded shows nothing.
  assert.equal(yardsAt(replyOf([], {}, {}, false)), null);
});

const line = (progress: string, cost: string | null) => ({
  id: '5', measures: { live: '1', progress, dockyards: '4', ...cost === null ? {} : { cost } }, queue: [], ships: [],
});

test('refit progress is accumulated IC over its cost, and dockyards are unscaled counts', () => {
  // Backend scales: progress and cost are fixed5 IC; dockyards and totals are plain integers.
  const early = yardsAt(replyOf([line('250000', '500000')], { dockyards: '4', censusComplete: '1' }, { ProductionLines: 0 }))!;
  const late = yardsAt(replyOf([line('250000', '0')], { dockyards: '4', censusComplete: '0' }, { ProductionLines: 0 }))!;
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
  const at = (progress: string, cost: string, dated: boolean) =>
    yardsAt(replyOf([line(progress, cost)], {}, dated ? { ProductionLines: 0 } : {}))!;
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
