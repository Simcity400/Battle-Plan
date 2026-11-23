import { datedToken } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { countryDrillTagAt } from './countryIdentity.ts';
import { chosenDatedCountry, datedCountrySpans, readDatedCountry } from './datedCountry.ts';

test('dated choices bind one native holder through renames and reject reuse, gaps and conflicting aliases', () => {
  const pick = { campaign: 'c', branch: 'b', tag: 'X', originOffset: 5 };
  const token = datedToken(pick);
  assert.deepEqual(readDatedCountry(token, 'c', 'b', { kind: 'tags', tags: ['X'] }), pick);
  assert.equal(readDatedCountry(token, 'other', 'b'), null);
  assert.equal(readDatedCountry(token, 'c', 'other'), null);
  assert.equal(readDatedCountry(token, 'c', 'b', { kind: 'tags', tags: ['Y'] }), null);
  const histories = [
    { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 120, slot: 2, countryId: 20 }] },
    { tag: 'Y', identities: [{ firstHour: 110, lastHour: 114, slot: 1, countryId: 10 }, { firstHour: 117, lastHour: 120, slot: 1, countryId: 10 }] },
  ];
  const spans = datedCountrySpans(histories, pick, 100, 120)!;
  assert.equal(countryDrillTagAt(spans, 105), 'X');
  assert.equal(countryDrillTagAt(spans, 112), 'Y');
  assert.equal(countryDrillTagAt(spans, 116), null);
  assert.equal(countryDrillTagAt(spans, 118), 'Y');
  assert.equal(datedCountrySpans([], pick, 100, 120), null, 'older missing identity prefixes cannot fall back to a tag');
  assert.equal(datedCountrySpans([...histories, { tag: 'Z', identities: [{ firstHour: 105, lastHour: 111, slot: 1, countryId: 10 }] }], pick, 100, 120), null);
});

test('alias membership clips a 5000-country catalogue without admitting the old tag replacement', () => {
  const histories = Array.from({ length: 5000 }, (_, slot) => ({ tag: `country:${slot}`, identities: [{ firstHour: 100, lastHour: 109, slot, countryId: slot + 10000 }] }));
  histories.push({ tag: 'renamed', identities: [{ firstHour: 110, lastHour: 120, slot: 4999, countryId: 14999 }] });
  histories[4999].identities.push({ firstHour: 110, lastHour: 120, slot: 5000, countryId: 15000 });
  const membership = new Map([['country:4999', [{ firstHour: 100, lastHour: 120 }]], ['renamed', [{ firstHour: 110, lastHour: 114 }]]]);
  const spans = datedCountrySpans(histories, { campaign: 'c', branch: 'b', tag: 'country:4999', originOffset: 5 }, 100, 120, membership)!;
  assert.deepEqual([...spans.keys()], ['country:4999', 'renamed']);
  assert.equal(countryDrillTagAt(spans, 112), 'renamed');
  assert.equal(countryDrillTagAt(spans, 118), null);
  assert.equal(countryDrillTagAt(datedCountrySpans(histories, { campaign: 'c', branch: 'b', tag: 'country:4999', originOffset: 5 }, 100, 120, new Map())!, 105), null);
});

test('a choice with nothing saved is its automatic one: the widget’s own country as it was when first shown; a saved one is read as saved', () => {
  const one = { kind: 'tags' as const, tags: ['GER'] };
  assert.deepEqual(chosenDatedCountry('', 'c', 'b', one, 'GER', 100, 200, 150), { campaign: 'c', branch: 'b', tag: 'GER', originOffset: 50 });
  assert.equal(chosenDatedCountry('', 'c', 'b', one, undefined, 100, 200, 150), null);
  assert.equal(chosenDatedCountry('', 'c', 'b', one, 'GER', 100, 200, 201), null, 'an hour outside the branch');
  const saved = { campaign: 'c', branch: 'b', tag: 'GER', originOffset: 7 };
  assert.deepEqual(chosenDatedCountry(datedToken(saved), 'c', 'b', one, 'ENG', 100, 200, 150), saved);
  assert.equal(chosenDatedCountry(datedToken({ ...saved, branch: 'other' }), 'c', 'b', one, 'GER', 100, 200, 150), null, 'a saved choice of another branch is not replaced by a guess');
});
