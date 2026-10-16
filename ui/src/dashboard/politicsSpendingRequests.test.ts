import assert from 'node:assert/strict';
import test from 'node:test';
import { loadProduct } from './products.ts';
import type { DashboardClient, DashboardSource } from './client';
import type { PoliticsLedgerReply } from '../generated/dashboard';
import { validatePowerBalance } from './powerBalance.ts';

test('5000 prepared ledger actors share repeated window requests without a branch scan or request loop', async () => {
  const source = { id: 'ledger-scale', revision: '1' } as DashboardSource;
  let requests = 0;
  const client = {
    product: async () => { throw new Error('Request-time ledger scan'); },
    preparedQuery: async (_source: DashboardSource, product: string, params: { branchId: string; tag: string }) => {
      assert.equal(product, 'politics-ledger');
      requests++;
      assert.ok(requests <= 5000, 'Unchanged replies must not start additional requests');
      return { branchId: params.branchId, tag: params.tag, decimals: 5, warnings: [], spending: [], spendItems: [], spendCategories: [],
        spendingCoverage: [], commandHolds: [], ideaTooltips: null, gains: [{ currency: 'Political', gainedRaw: '9007199254740993', lostRaw: '7', complete: true }] };
    },
  } as unknown as DashboardClient;
  for (let index = 0; index < 5000; index++) {
    const request = { sourceId: source.id, revision: source.revision, product: 'politics-ledger',
      params: { branchId: '1', tag: `mod actor ${index}`, view: 'spending', from: 24, hour: 29, totals: true } };
    const signal = new AbortController().signal;
    const replies = await Promise.all(Array.from({ length: 3 }, () => loadProduct<PoliticsLedgerReply>(client, request, signal, validatePowerBalance, source)));
    assert.ok(replies.every(reply => reply === replies[0]));
    assert.equal(replies[0].tag, request.params.tag);
    assert.equal(replies[0].gains?.[0].gainedRaw, '9007199254740993');
  }
  assert.equal(requests, 5000);
});
