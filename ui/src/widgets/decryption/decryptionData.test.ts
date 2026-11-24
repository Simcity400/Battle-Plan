import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';
import type { CipherPair, DecryptionReply } from '../../generated/dashboard';
import { activeBonusDays, amountText, bonusText, cipherNumbers, pairKey, percentOf, progressLabel, progressPercent, progressText, raceRows, raceStructure, validateDecryption } from './decryptionData.ts';

const pair = (owner: string | null, target: string, id: number, thresholdRaw: string | null = '1000000', extra: Partial<CipherPair> = {}): CipherPair => ({
  owner: { id, tag: owner }, target: { id: id + 100, tag: target }, thresholdRaw, thresholdUnknown: thresholdRaw === null ? 'The target RaidIntelInputs row is not recorded' : null,
  bonusActive: 0, decrypting: 1, broken: false, started: { hour: 100, upperBound: false }, brokenAt: null, ...extra,
});
const reply = (pairs: CipherPair[], attemptsUnknown: string | null = null): DecryptionReply => ({ pairs, missing: null, attemptsUnknown, warnings: [] });
const values = (cells: Record<string, number>) => valuesOf(new Map(Object.entries(cells)), 'Not read yet');
const numbers = (item: CipherPair, amount?: number, bonusDays?: number) => cipherNumbers(values({
  ...(amount === undefined ? {} : { [liveKey(pairKey(item), 'amount')]: amount }), ...(bonusDays === undefined ? {} : { [liveKey(pairKey(item), 'bonusDays')]: bonusDays }),
}), item);

test('the meter percent matches the game exactly: rounding, wrapping, the sentinel and overflow', () => {
  const vectors = JSON.parse(readFileSync(new URL('../../../../crates/dashboard-text/src/cryptology/test-values.json', import.meta.url), 'utf8')) as [string, number, string, number | null][];
  for (const [amount, active, threshold, expected] of vectors) {
    assert.equal(progressPercent(BigInt(amount), active, BigInt(threshold)), expected, `${amount}/${threshold}, active ${active}`);
  }
});

test('progress is painted from the live amount over the threshold, and unknown where either is', () => {
  const known = pair('GER', 'ENG', 1);
  assert.equal(percentOf(known, numbers(known, 500_000)), 50);
  assert.equal(progressLabel(percentOf(known, numbers(known, 500_000))), '50%');
  assert.equal(percentOf(known, numbers(known)), null);
  assert.equal(amountText(numbers(known)), 'Amount not read (Not read yet)');
  const unknown = pair('GER', 'ENG', 1, null);
  assert.equal(percentOf(unknown, numbers(unknown, 400_000)), null);
  assert.equal(progressText(null, numbers(unknown, 400_000)), 'Progress unknown · 4');
  assert.equal(amountText(numbers(unknown, 400_001)), '4.00001');
  assert.equal(amountText(numbers(unknown, -400_001)), '−4.00001');
});

test('the bonus countdown shows only on a broken cipher whose bonus is active, keeping its sign', () => {
  const broken = pair('ITA', 'FRA', 4, null, { broken: true, bonusActive: 7, brokenAt: { hour: 120, upperBound: true } });
  assert.equal(activeBonusDays(broken, numbers(broken, 0, 23)), 23);
  assert.equal(activeBonusDays(broken, numbers(broken, 0, -1)), -1);
  assert.equal(bonusText(-1), '-1 bonus days');
  assert.equal(bonusText(23), '+23 bonus days');
  assert.equal(activeBonusDays({ ...broken, bonusActive: 0 }, numbers(broken, 0, 23)), null);
  assert.equal(activeBonusDays({ ...broken, broken: null }, numbers(broken, 0, 23)), null);
  assert.equal(activeBonusDays(broken, numbers(broken, 0)), null);
});


test('the race keeps the reply order; decryptor order groups by decryptor; either side filters, a side with no tag never matches', () => {
  const pairs = [pair('SOV', 'GER', 3), pair('GER', 'ENG', 1), pair(null, 'GER', 2), pair('ENG', 'GER', 5)];
  const data = reply(pairs);
  assert.deepEqual(raceRows(data, 'progress', 'decryptor', 0).map(row => row.owner.id), [3, 1, 2, 5]);
  assert.deepEqual(raceRows(data, 'decryptor', 'decryptor', 0).map(row => row.owner.id), [2, 5, 1, 3]);
  const germany = (tag: string) => tag === 'GER';
  assert.deepEqual(raceRows(data, 'progress', 'target', 0, germany).map(row => row.owner.id), [3, 2, 5]);
  assert.deepEqual(raceRows(data, 'progress', 'decryptor', 0, germany).map(row => row.owner.id), [1]);
});

test('the moving numbers are not in the structure, so a day of progress renders nothing', () => {
  const data = reply([pair('GER', 'ENG', 1)]);
  assert.equal(raceStructure(data), raceStructure(validateDecryption(JSON.parse(JSON.stringify(data)))));
});

test('a reply whose threshold is neither known nor named unknown is refused', () => {
  const good = reply([pair('GER', 'ENG', 1)]);
  assert.equal(validateDecryption(good), good);
  assert.throws(() => validateDecryption({ ...good, pairs: [{ ...good.pairs[0], thresholdRaw: null, thresholdUnknown: null }] }));
  assert.throws(() => validateDecryption({ ...good, pairs: [{ ...good.pairs[0], started: { hour: 1 } }] }));
});

test('the endpoint filter is hidden for All while keeping its saved option for a narrowed selection', () => {
  const options = resolveOptions(OPTIONS, { countrySide: 'target' });
  assert.deepEqual(sectionFields(OPTIONS, 'countries', options, { kind: 'all' }), []);
  assert.deepEqual(sectionFields(OPTIONS, 'countries', options, { kind: 'tags', tags: ['Mod target'] }).map(field => field.key), ['countrySide']);
  assert.equal(options.countrySide, 'target');
});
