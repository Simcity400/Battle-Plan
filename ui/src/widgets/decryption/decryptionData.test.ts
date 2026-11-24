import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';
import type { CipherPair, DecryptionReply } from '../../generated/dashboard';
import { activeBonusDays, amountText, percentOf, progressLabel, progressPercent, validateDecryption, weekChangeText, type CipherNumbers } from './decryptionData.ts';

const pair = (owner: string | null, target: string, id: number, thresholdRaw: string | null = '1000000', extra: Partial<CipherPair> = {}): CipherPair => ({
  owner: { id, tag: owner }, target: { id: id + 100, tag: target }, thresholdRaw, thresholdUnknown: thresholdRaw === null ? 'The target RaidIntelInputs row is not recorded' : null,
  bonusActive: 0, decrypting: 1, broken: false, started: { hour: 100, upperBound: false }, brokenAt: null, weekChangeTenths: 24, weekChangeUnknown: null, ...extra,
});
const reply = (pairs: CipherPair[], attemptsUnknown: string | null = null): DecryptionReply => ({ pairs, missing: null, attemptsUnknown, warnings: [] });
const numbers = (_item: CipherPair, amount?: number, bonusDays?: number): CipherNumbers => ({ amount, bonusDays, missing: amount === undefined ? 'Not read yet' : '' });

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
  assert.equal(amountText(numbers(unknown, 400_001)), '4.00001');
  assert.equal(amountText(numbers(unknown, -400_001)), '−4.00001');
});

test('the bonus countdown shows only on a broken cipher whose bonus is active, keeping its sign', () => {
  const broken = pair('ITA', 'FRA', 4, null, { broken: true, bonusActive: 7, brokenAt: { hour: 120, upperBound: true } });
  assert.equal(activeBonusDays(broken, numbers(broken, 0, 23)), 23);
  assert.equal(activeBonusDays(broken, numbers(broken, 0, -1)), -1);
  assert.equal(activeBonusDays({ ...broken, bonusActive: 0 }, numbers(broken, 0, 23)), null);
  assert.equal(activeBonusDays({ ...broken, broken: null }, numbers(broken, 0, 23)), null);
  assert.equal(activeBonusDays(broken, numbers(broken, 0)), null);
});


test('a reply whose threshold is neither known nor named unknown is refused', () => {
  const good = reply([pair('GER', 'ENG', 1)]);
  assert.equal(validateDecryption(good), good);
  assert.throws(() => validateDecryption({ ...good, pairs: [{ ...good.pairs[0], thresholdRaw: null, thresholdUnknown: null }] }));
  assert.throws(() => validateDecryption({ ...good, pairs: [{ ...good.pairs[0], started: { hour: 1 } }] }));
  assert.throws(() => validateDecryption({ ...good, pairs: [{ ...good.pairs[0], weekChangeTenths: null, weekChangeUnknown: null }] }), 'a weekly change neither known nor named unknown is refused');
  assert.equal(validateDecryption({ ...good, pairs: [{ ...good.pairs[0], weekChangeTenths: null, weekChangeUnknown: 'Not recorded 7 days earlier' }] }).pairs.length, 1);
  assert.deepEqual([weekChangeText(24), weekChangeText(-5), weekChangeText(0)], ['+2.4 points', '−0.5 points', '0.0 points']);
});

test('the endpoint filter is hidden for All while keeping its saved option for a narrowed selection', () => {
  const options = resolveOptions(OPTIONS, { countrySide: 'target' });
  assert.deepEqual(sectionFields(OPTIONS, 'countries', options, { kind: 'all' }), []);
  assert.deepEqual(sectionFields(OPTIONS, 'countries', options, { kind: 'tags', tags: ['Mod target'] }).map(field => field.key), ['countrySide']);
  assert.equal(options.countrySide, 'target');
});
