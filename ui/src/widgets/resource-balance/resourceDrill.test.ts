import assert from 'node:assert/strict';
import test from 'node:test';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';
import { countryDrillTagAt } from '../../dashboard/countryIdentity.ts';
import { runAt } from '../../charts/series.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';
import type { CountryIdentityHistory, ResourceBalanceReply, TradeRun } from '../../generated/dashboard';
import { clipResourceTrades, readResourceCountry, resourceCountryData, resourceCountrySpans, resourceCountryToken, resourceExtent, resourceMoment } from './resourceDrill.ts';

const selection = { campaign: 'c', branch: 'b', tag: 'X', originOffset: 5, extent: [2, 18] as [number, number] };
const identities: CountryIdentityHistory[] = [
  { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 120, slot: 2, countryId: 20 }] },
  { tag: 'Y', identities: [{ firstHour: 110, lastHour: 114, slot: 1, countryId: 10 }, { firstHour: 117, lastHour: 120, slot: 3, countryId: 30 }] },
];
const deal = (id: string, firstHour = 100, lastHour = 120): TradeRun => ({ deal: id, firstHour, lastHour, startsSegment: true,
  resource: 7, direction: 'import', partner: 'P', deliveredRaw: '100000', factories: 1, convoysAssigned: 0, convoysRequired: 0 });
function reply(tag: string, value: string): ResourceBalanceReply {
  return { tag, branchId: 'b', decimals: 5, warnings: [], resources: [{ id: 7, token: 'mod-resource', ordinal: 1, name: null, icon: null }],
    series: [{ id: 7, runs: [{ firstHour: 100, lastHour: 120, startsSegment: true, producedRaw: value, subjectsRaw: '0', importedRaw: '0',
      toExportRaw: '0', exportedRaw: '0', availableRaw: value, afterProjectsRaw: value, productionRaw: '0' }] }],
    trades: [deal(`${tag}-1`), deal(`${tag}-2`)], dealHours: [{ firstHour: 100, lastHour: 120 }] };
}

test('a dated resource pin follows its native country through renames and refuses both tag replacements', () => {
  const restored = readResourceCountry(resourceCountryToken(selection), 'c', 'b', { kind: 'tags', tags: ['X'] })!;
  assert.deepEqual(restored, selection);
  assert.equal(resolveOptions(OPTIONS, { resourceCountry: resourceCountryToken(selection) }).resourceCountry, resourceCountryToken(selection));
  assert.deepEqual(resourceExtent(restored, 100, 120), [102, 118]);
  const spans = resourceCountrySpans(identities, restored, 100, 120)!;
  assert.equal(countryDrillTagAt(spans, 105), 'X');
  assert.equal(countryDrillTagAt(spans, 112), 'Y');
  assert.equal(countryDrillTagAt(spans, 116), null);
  assert.equal(countryDrillTagAt(spans, 119), null);
  const data = resourceCountryData([reply('X', '1100000'), reply('Y', '1300000')], spans, 'X')!;
  assert.equal(runAt(data.series[0].runs, 105)?.producedRaw, '1100000');
  assert.equal(runAt(data.series[0].runs, 112)?.producedRaw, '1300000');
  assert.equal(runAt(data.series[0].runs, 119), null);
  assert.equal(data.trades.filter(deal => deal.firstHour <= 112 && deal.lastHour >= 112).length, 2);
  assert.equal(readResourceCountry(resourceCountryToken(selection), 'other', 'b', { kind: 'tags', tags: ['X'] }), null);
  assert.equal(readResourceCountry(resourceCountryToken(selection), 'c', 'other', { kind: 'tags', tags: ['X'] }), null);
  assert.equal(readResourceCountry(resourceCountryToken(selection), 'c', 'b', { kind: 'tags', tags: ['Y'] }), null);
  assert.equal(resourceCountrySpans([], selection, 100, 120), null);
  assert.equal(resourceCountrySpans(identities, { ...selection, originOffset: 21 }, 100, 120), null);
});

test('five thousand overlapping deals retain every member-hour contribution', () => {
  const trades = Array.from({ length: 5000 }, (_, index) => deal(String(index)));
  const clipped = clipResourceTrades(trades, [{ firstHour: 103, lastHour: 105 }, { firstHour: 110, lastHour: 114 }]);
  assert.equal(clipped.length, 10000);
  assert.equal(clipped.reduce((sum, trade) => sum + BigInt(trade.deliveredRaw) * BigInt(trade.lastHour - trade.firstHour + 1), 0n), 4000000000n);
  assert.equal(new Set(clipped.map(trade => trade.deal)).size, 5000);
  assert.ok(clipped.every(trade => !trade.startsSegment));
});

test('a smart-group drill uses each native alias membership and preserves leaving and missing evidence', () => {
  const members = new Map([['X', [{ firstHour: 100, lastHour: 109 }]], ['Y', [{ firstHour: 110, lastHour: 112 }]]]);
  const spans = resourceCountrySpans(identities, selection, 100, 120, members)!;
  assert.equal(countryDrillTagAt(spans, 105), 'X');
  assert.equal(countryDrillTagAt(spans, 111), 'Y');
  assert.equal(countryDrillTagAt(spans, 113), null, 'leaving the group cuts history');
  assert.equal(countryDrillTagAt(spans, 119), null, 'a member reusing X never substitutes for the native holder');
  const data = resourceCountryData([reply('X', '1100000'), reply('Y', '1300000')], spans, 'X')!;
  assert.equal(runAt(data.series[0].runs, 111)?.producedRaw, '1300000');
  assert.equal(runAt(data.series[0].runs, 113), null);
  assert.equal(resourceCountrySpans(identities, selection, 100, 120, new Map())!.size, 0);
});

test('resource breakdown initial Pin, local history picks and Follow use their actual displayed dates', () => {
  assert.equal(resourceMoment(CLOSED, true, 105, 112), 105);
  let drill = reduceDrill(CLOSED, { type: 'open', breakdown: 'resource', hour: 105, playing: true, playhead: null });
  assert.equal(resourceMoment(drill, false, 120, 112), 105, 'a held main view opens at its drawn hour');
  drill = reduceDrill(drill, { type: 'follow' });
  assert.equal(resourceMoment(drill, true, 105, 112), 112);
  drill = reduceDrill(drill, { type: 'pick', hour: 104 });
  assert.equal(resourceMoment(drill, true, 105, 113), 104);
});
