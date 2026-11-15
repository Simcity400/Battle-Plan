import assert from 'node:assert/strict';
import test from 'node:test';
import { nameKey } from '../../dashboard/countryFacts.ts';
import type { CountryStyle, TradeFlowsReply } from '../../generated/dashboard';
import { decodeTrade } from './tradeData.ts';
import { heldTradeColours, tradeColourPlan, tradeCountryColours, tradeFixedColourKey } from './tradeColours.ts';

const identity = (slot: number) => JSON.stringify([slot, slot + 1]);
const branch = { firstHour: 0, lastHour: 2000, gaps: [] };
function data(count: number) {
  const countries = Array.from({ length: count }, (_, slot) => `mod-country-${slot}`);
  return decodeTrade({ branchId: 'b', decimals: 0, countries,
    countryIdentities: countries.map((tag, slot) => ({ tag, runs: [{ firstHour: 0, lastHour: 2000, slot, countryId: slot + 1 }] })),
    resources: [], firstHour: 0, lastHour: 2000, known: [{ firstHour: 0, lastHour: 2000 }], partial: [], warnings: [], flows: [] } as TradeFlowsReply);
}
function styles(plan: ReturnType<typeof tradeColourPlan>) {
  return new Map<string, CountryStyle>(plan.wanted.map(sample => [nameKey(sample.hour, sample.tag),
    { tag: sample.tag, name: null, color: [136, 136, 136] }]));
}

test('5,000 native holder slots stay complete and ordered while only full style batches admit correction', () => {
  const trade = data(5000);
  const first = tradeColourPlan(trade, branch, 10, true);
  assert.equal(first.selection.size, 5000);
  assert.equal(first.wanted.length, 5000);
  const partial = new Map([...styles(first)].slice(0, 64));
  assert.equal(tradeFixedColourKey(first, partial), null);
  const key = tradeFixedColourKey(first, styles(first));
  assert.equal(JSON.parse(key!).length, 5000);
  // A new dated style sample with unchanged native colours needs no correction.
  const later = tradeColourPlan(trade, branch, 730, true);
  assert.equal(tradeFixedColourKey(later, styles(later)), key);
  assert.equal(tradeColourPlan(trade, branch, 10, false).wanted.length, 0);
});

test('one native holder colour is shared across both sides and does not depend on positive node ranks', () => {
  const trade = data(3);
  const plan = tradeColourPlan(trade, branch, 10, true);
  const key = tradeFixedColourKey(plan, styles(plan))!;
  const corrected = tradeCountryColours(key, '#111111', true);
  const firstFrame = [identity(0), identity(1), identity(0)].map(holder => corrected.get(holder));
  const reranked = [identity(0), identity(0), identity(1)].map(holder => corrected.get(holder));
  assert.equal(firstFrame[0], firstFrame[2]);
  assert.equal(firstFrame[0], reranked[0]);
  assert.equal(firstFrame[1], reranked[2]);
  assert.notEqual(corrected.get(identity(0)), corrected.get(identity(1)));
  const raw = tradeCountryColours(key, '#111111', false);
  assert.equal(raw.get(identity(0)), '#888888');
  assert.equal(raw.get(identity(1)), '#888888');
});

test('reused aliases keep dated native holders separate and pending colours never cross an alias or recording gap', () => {
  const trade = decodeTrade({ branchId: 'b', decimals: 0, countries: ['same-tag'],
    countryIdentities: [{ tag: 'same-tag', runs: [{ firstHour: 0, lastHour: 23, slot: 0, countryId: 1 },
      { firstHour: 24, lastHour: 100, slot: 1, countryId: 2 }] }],
    resources: [], firstHour: 0, lastHour: 2000, known: [{ firstHour: 0, lastHour: 100 }], partial: [], warnings: [], flows: [] } as TradeFlowsReply);
  const plan = tradeColourPlan(trade, branch, 80, true);
  assert.deepEqual(plan.wanted.map(sample => sample.hour), [0, 24]);
  const answered = styles(plan);
  answered.delete(nameKey(24, 'same-tag'));
  assert.equal(tradeFixedColourKey(plan, answered), null);
  const colours = new Map([[identity(0), '#123456'], [identity(1), '#654321']]);
  const later = new Map(plan.selection);
  later.set(identity(0), { tag: 'new-alias', hour: 100, stretch: 100 });
  assert.deepEqual([...heldTradeColours(colours, plan.selection, later)], [[identity(1), '#654321']]);
  later.set(identity(1), { tag: 'same-tag', hour: 120, stretch: 120 });
  assert.equal(heldTradeColours(colours, plan.selection, later).size, 0);
});
