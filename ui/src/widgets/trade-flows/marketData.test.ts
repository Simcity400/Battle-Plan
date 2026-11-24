import test from 'node:test';
import assert from 'node:assert/strict';
import { rawAmount, validateMarketReply } from './marketData.ts';
test('market and lend-lease preserves complete fixed point values and fuel scales', () => {
  assert.equal(rawAmount('9007199254740993'), '90071992547.40993');
  assert.equal(rawAmount('-1'), '-0.00001');
  assert.equal(rawAmount('32769', 32768n), '1.00003');
  assert.equal(rawAmount('10001', 1000n), '10.001');
});

test('market unavailable collections remain unknown and malformed nested values refuse', () => {
  const reply = { branchId: '1', offers: null, contracts: [], agreements: [], purchasesMissing: null, unattributed: 0, variants: [], icons: [], warnings: ['writer unavailable'] };
  assert.equal(validateMarketReply(reply).offers, null);
  assert.throws(() => validateMarketReply({ ...reply, offers: [{ country: 5000, tag: 'AA', inventory: [{ variant: '1', name: null, amountRaw: 1 }], prices: [], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false }] }));
  assert.throws(() => validateMarketReply({ ...reply, unattributed: undefined }), 'a reply says how many rows it could not attribute');
});


