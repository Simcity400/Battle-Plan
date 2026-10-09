import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryStabilityReply, PoliticsLedgerReply } from '../../generated/dashboard';
import { powerLedgerModelKey, powerMaximaModel } from './powerPickModel.ts';

const maxima = (): CountryStabilityReply => ({ branchId: 'branch', decimals: 5, warnings: [], series: [{
  tag: 'recorded actor', stability: [], warSupport: [], recorded: true, breakdown: null,
  maxCommandPower: [{ firstHour: 24, lastHour: 48, valueRaw: '10000000', startsSegment: true }],
  carried: [{ firstHour: 25, lastHour: 48, readHour: 24 }],
}] });
const ledger = (): PoliticsLedgerReply => ({ branchId: 'branch', tag: 'recorded actor', decimals: 5, warnings: [],
  daily: [{ country: 1, instance: 1, epoch: '1', dayHour: 24, readHour: 26, ppReadHour: 24, cpReadHour: 26,
    ppGrossRaw: '200000', ppCostRaw: '50000', ppIncomeRaw: '150000', ppRealizedRaw: '150000', ppSequence: '1',
    cpIncomeRaw: '100000', cpRealizedRaw: '100000', cpSequence: '2' }],
  spending: [{ id: 'payment', sequence: '3', ordinal: 0, hour: 24, readHour: 24, countryTag: 'recorded actor', countryName: 'Old name',
    currency: 'Political', category: 'recorded category', itemId: 'item', chargeRaw: '40000', realizedDeltaRaw: '-40000',
    covered: true, occurrences: 1, changes: [{ hour: 24, readHour: 24, chargeRaw: '40000', realizedDeltaRaw: '-40000', covered: true, occurrences: 1 }] }],
  spendItems: [{ id: 'item', groupId: 'old presentation group', name: 'Old item', action: 'Old action', icon: null,
    tooltipToken: null, character: null, targetTag: null, targetName: null, unknown: null }],
  spendCategories: [{ id: 'recorded category', name: 'Old category', identified: true }],
  spendingCoverage: [{ readHour: 24, sinceHour: 24, throughHour: 48, political: true, command: true, reason: null }],
  balanceChanges: [{ hour: 24, readHour: 24, currency: 'Political', amountRaw: '-40000', realizedDeltaRaw: '-40000',
    covered: true, action: 'Old cause', reason: null }],
  commandHolds: [{ readHour: 24, throughHour: 48, totalRaw: '200000', reason: null,
    holders: [{ id: 'native holder', kind: 'native domain', name: 'Old holder', amountRaw: '200000' }] }], ideaTooltips: null,
});

test('maximum ownership excludes content presentation but retains numeric scale, values and supplying dates', () => {
  const original = maxima();
  const refreshed = { ...maxima(), warnings: ['Changed content warning'], series: maxima().series.map(row => ({ ...row, recorded: false })) };
  assert.equal(powerMaximaModel(refreshed).key, powerMaximaModel(original).key);
  for (const changed of [
    { ...maxima(), decimals: 4 },
    { ...maxima(), series: maxima().series.map(row => ({ ...row, maxCommandPower: [{ firstHour: 24, lastHour: 48, valueRaw: '9000000', startsSegment: true }] })) },
    { ...maxima(), series: maxima().series.map(row => ({ ...row, carried: [{ firstHour: 25, lastHour: 48, readHour: 23 }] })) },
  ]) assert.notEqual(powerMaximaModel(changed).key, powerMaximaModel(original).key);
});

test('ledger ownership survives text, art, refusal and content-group refreshes', () => {
  const original = ledger();
  const refreshed = ledger();
  refreshed.warnings = ['New content warning'];
  refreshed.spending[0].countryName = 'New country';
  refreshed.spendItems[0] = { ...refreshed.spendItems[0], name: 'New item', action: 'New action', icon: 'new art',
    groupId: 'new presentation group', tooltipToken: 'new tooltip', unknown: 'New named content refusal' };
  refreshed.spendCategories[0] = { ...refreshed.spendCategories[0], name: 'New category', identified: false };
  refreshed.spendingCoverage[0].reason = 'New translated coverage reason';
  refreshed.balanceChanges![0].action = 'New cause';
  refreshed.commandHolds[0].holders[0].name = 'New holder';
  assert.equal(powerLedgerModelKey(refreshed), powerLedgerModelKey(original));
});

test('ledger ownership changes for native dates, operands, payment contributions, coverage and holds', () => {
  const original = powerLedgerModelKey(ledger());
  const changes: ((reply: PoliticsLedgerReply) => void)[] = [
    reply => { reply.daily![0].ppReadHour = 25; },
    reply => { reply.daily![0].cpIncomeRaw = null; },
    reply => { reply.spending[0].changes[0].hour = 25; },
    reply => { reply.spending[0].changes[0].chargeRaw = '50000'; },
    reply => { reply.spendingCoverage[0].throughHour = 47; },
    reply => { reply.balanceChanges![0].realizedDeltaRaw = '0'; },
    reply => { reply.commandHolds[0].holders[0].amountRaw = '100000'; },
  ];
  for (const change of changes) {
    const next = ledger(); change(next);
    assert.notEqual(powerLedgerModelKey(next), original);
  }
});
