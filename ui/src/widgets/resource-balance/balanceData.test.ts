import test from 'node:test';
import assert from 'node:assert/strict';
import { contentSet, parseStoredOptions, resolveOptions, UNSTORABLE } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';
import type { PoolRun, ResourceBalanceReply, TradeRun } from '../../generated/dashboard';
import { clipToSpans, runAt } from '../../charts/series.ts';
import { tooltipHeight, tooltipRoom, tooltipText } from '../../charts/tooltip.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { HourStore, planRequests } from '../../dashboard/countryFacts.ts';
import { automaticResource, balanceOf, barScale, breakdownRows, changeHours, chosenLines, chosenResources, dealsKnown, FULL_BREAKDOWN_MAX, MAX_DEALS, orderResources, overTimeTooltip,
  resourceName, resourcePartGroups, resourceParts, resourceTooltip, supplyDemandRuns, toggleChosen, tooltipPartners, tradeIndex, tradesAt, validateResourceBalanceReply } from './balanceData.ts';
import { visibleRange } from '../../components/countryList.ts';
import { dealAccounting, dealWindow } from './resourceDeals.ts';

const U = 100000;

test('automatic resources can choose an unprepared history before any exact pool operands arrive', () => {
  const summary = { from: 0, to: 29, width: 1, approximate: [{ firstHour: 0, lastHour: 29 }], buckets: [{ firstHour: 0, lastHour: 29,
    firstValue: 3, lastValue: 3, minValue: 3, maxValue: 3, minHour: 0, maxHour: 0, lastValueHour: 0, firstBreak: true, hasBreak: false }] };
  const data = { branchId: '1', tag: 'C', decimals: 5, warnings: [], resources: [{ id: 7, token: 'mod/native', ordinal: 1, name: null, icon: null }],
    series: [{ id: 7, runs: [] }], trades: [], dealHours: [], campaignSizes: {}, summaries: [{ id: 7, supply: summary, demand: summary }] };
  assert.equal(automaticResource(data), 'mod/native');
  assert.equal(data.series[0].runs.length, 0, 'geometry never becomes a written exact pool amount');
});

test('signed resource parts preserve native net even when availability has a rounding residual', () => {
  const run = pools(1, 2, { producedRaw: 9, importedRaw: 2, subjectsRaw: -1, toExportRaw: 3, availableRaw: 7.00001,
    afterProjectsRaw: 5.00001, productionRaw: -4 });
  const parts = resourceParts(run);
  assert.equal(parts.at(-1)!.raw, 1n);
  assert.equal(parts.reduce((total, part) => total + part.raw, 0n), balanceOf(run).net);
  assert.equal(parts.find(part => part.id === 'projects')!.raw, -200000n);
  assert.equal(parts.find(part => part.id === 'overlord')!.raw, -100000n);
});
/** Germany's steel in the part-G save (per day): produced 301, reserve 75, 72 bought, lines need 30. */
const pools = (firstHour: number, lastHour: number, values: Partial<Record<keyof PoolRun, number>>, startsSegment = true): PoolRun => {
  const raw = (key: keyof PoolRun) => String(Math.round((values[key] as number | undefined ?? 0) * U));
  return { firstHour, lastHour, startsSegment, producedRaw: raw('producedRaw'), subjectsRaw: raw('subjectsRaw'), importedRaw: raw('importedRaw'),
    toExportRaw: raw('toExportRaw'), exportedRaw: raw('exportedRaw'), availableRaw: raw('availableRaw'), afterProjectsRaw: raw('afterProjectsRaw'),
    productionRaw: raw('productionRaw') };
};
const STEEL = pools(100, 123, { producedRaw: 301, toExportRaw: 75, exportedRaw: 72, availableRaw: 226, afterProjectsRaw: 226, productionRaw: -30 });
const RUBBER = pools(100, 123, { productionRaw: -5 });
const trade = (partner: string, direction: 'import' | 'export', delivered: number, resource = 5): TradeRun => ({
  deal: `${partner}${direction}`, resource, direction, partner, deliveredRaw: String(delivered * U), factories: 1, convoysAssigned: 0, convoysRequired: 0,
  firstHour: 100, lastHour: 123, startsSegment: true });
const REPLY: ResourceBalanceReply = {
  branchId: '1', tag: 'GER', decimals: 5, warnings: [], dealHours: [{ firstHour: 100, lastHour: 130 }],
  // Resources are told apart by their native index (steel 5, rubber 7), not their token.
  resources: [{ id: 5, token: 'steel', ordinal: 1, name: 'Steel', icon: null }, { id: 7, token: 'rubber', ordinal: 1, name: null, icon: null }],
  series: [{ id: 5, runs: [STEEL, pools(124, 130, { producedRaw: 320, availableRaw: 320, afterProjectsRaw: 316, productionRaw: -30 }, false)] },
    { id: 7, runs: [RUBBER] }],
  trades: [trade('SPR', 'export', 40), trade('SAU', 'export', 8), trade('ITA', 'import', 2, 7)],
};

test('moment groups preserve negative supply and reversed demand operands', () => {
  const run = pools(100, 123, { producedRaw: -2, importedRaw: -3, toExportRaw: -4, availableRaw: -1, afterProjectsRaw: 1, productionRaw: 2 });
  const groups = resourcePartGroups(run);
  assert.equal(groups[0].values.find(part => part.id === 'produced')!.raw, -200000n);
  assert.equal(groups[0].values.find(part => part.id === 'imported')!.raw, -300000n);
  assert.equal(groups[1].values.find(part => part.id === 'projects')!.raw, -200000n);
  assert.equal(groups[1].values.find(part => part.id === 'production')!.raw, -200000n);
  assert.equal(groups[1].values.find(part => part.id === 'exported')!.raw, -400000n);
  for (const group of groups) assert.equal(group.values.reduce((sum, part) => sum + part.raw, 0n), group.total);
});

test('five thousand deals keep all exact accounting while only visible rows ask for partners', () => {
  const deals = Array.from({ length: 5000 }, (_, index) => trade(`C${index}`, 'import', index === 4999 ? -1 : 2));
  assert.equal(dealAccounting(deals).total, 9997n * BigInt(U));
  assert.equal(dealAccounting(deals).widest, 2n * BigInt(U));
  const range = visibleRange(deals.length, 4990 * 32, 240, 32, 0);
  const window = dealWindow(deals, range, 110);
  assert.equal(window.wanted.length, window.shown.length);
  assert.ok(window.wanted.length < 10);
  assert.ok(window.wanted.some(partner => partner.tag === 'C4997'));
  const last = dealWindow(deals, visibleRange(deals.length, 4999 * 32, 240, 32, 0), 110);
  assert.equal(last.shown.at(-1)!.deliveredRaw, String(-U));
  assert.equal(last.wanted.at(-1)!.tag, 'C4999');
});

test('the game’s surplus is what is left after projects minus what the lines need', () => {
  const steel = balanceOf(STEEL);
  assert.equal(steel.supply, 301n * 100000n);
  assert.equal(steel.demand, 105n * 100000n);
  assert.equal(steel.net, 196n * 100000n);
  // Lines needing rubber the country lacks are a deficit, not zero.
  assert.equal(balanceOf(RUBBER).net, -5n * 100000n);
  const later = balanceOf(REPLY.series[0].runs[1]);
  assert.equal(later.projects, 4n * 100000n);
  assert.equal(later.net, 286n * 100000n);
  // A subject's transfer to its overlord is demand; the overlord's receipt is supply.
  assert.equal(balanceOf(pools(0, 0, { producedRaw: 43, subjectsRaw: -4.3 })).demand, 430000n);
  assert.equal(balanceOf(pools(0, 0, { subjectsRaw: 4.3 })).supply, 430000n);
});

test('the breakdown lists each partner, the unattributed rest and the game’s total', () => {
  const rows = breakdownRows(balanceOf(STEEL), tradesAt(tradeIndex(REPLY), 5, 110), 5, tag => ({ name: `${tag} name`, flag: `flag:${tag}` }));
  const shown = rows.map(row => [row.label, row.value]);
  assert.deepEqual(shown, [
    ['Produced', '301'], ['Imported', '0'], ['Exported (trade law)', '−75'],
    ['SPR name · 1 civilian factory', '40'], ['SAU name · 1 civilian factory', '8'],
    // 72 were bought but the deals list 48: the rest is its own part.
    ['Not from a listed deal', '+24'], ['Set aside, not bought', '3'],
    ['Available', '226'], ['Needed by production lines', '−30'], ['Surplus', '+196'],
  ]);
  assert.equal(rows[3].countryFlag?.flag, 'flag:SPR');
  assert.equal(rows[4].countryFlag?.flag, 'flag:SAU');
  assert.equal(rows.at(-1)!.tone, 'positive');
  const rubber = breakdownRows(balanceOf(RUBBER), tradesAt(tradeIndex(REPLY), 7, 110), 5, tag => ({ name: tag }));
  // 2 delivered by a deal but nothing imported: the pool is what the country has.
  assert.deepEqual(rubber.filter(row => row.indent).map(row => [row.label, row.value]), [['ITA · 1 civilian factory', '2'], ['Not from a listed deal', '−2']]);
  assert.equal(rubber.at(-1)!.label, 'Deficit');
  assert.equal(rubber.at(-1)!.tone, 'negative');
});

test('resource partners retain their pending and refused flag status without changing delivered amounts', () => {
  const pending = { name: 'SPR name', flagPending: true };
  const refused = { name: 'SAU name', flagPending: false, flagReason: 'Recorded flag unavailable' };
  const rows = breakdownRows(balanceOf(STEEL), tradesAt(tradeIndex(REPLY), 5, 110), 5, tag => {
    assert.ok(tag === 'SPR' || tag === 'SAU');
    return tag === 'SPR' ? pending : refused;
  });
  assert.deepEqual(rows[3].countryFlag, pending);
  assert.equal(rows[3].value, '40');
  assert.deepEqual(rows[4].countryFlag, refused);
  assert.equal(rows[4].value, '8');
  assert.equal(rows[3].icon, undefined);
  assert.equal(rows[4].icon, undefined);
});

test('rows can follow the game’s order, the biggest shortfall or the largest volume', () => {
  assert.deepEqual(orderResources(REPLY, 110, 'game').map(item => item.token), ['steel', 'rubber']);
  assert.deepEqual(orderResources(REPLY, 110, 'shortfall').map(item => item.token), ['rubber', 'steel']);
  assert.deepEqual(orderResources(REPLY, 110, 'volume').map(item => item.token), ['steel', 'rubber']);
  // A resource not recorded at the hour goes last.
  assert.deepEqual(orderResources(REPLY, 126, 'shortfall').map(item => item.token), ['steel', 'rubber']);
});

test('the bar scale spans the campaign or the moment, and the automatic choice is the largest resource', () => {
  assert.equal(barScale(REPLY.series, null), 320n * 100000n);
  assert.equal(barScale(REPLY.series, 110), 301n * 100000n);
  assert.equal(barScale(REPLY.series, 99), 0n);
  assert.equal(automaticResource(REPLY), 'steel');
  assert.deepEqual(chosenResources(REPLY, []), ['steel']);
  assert.deepEqual(chosenResources(REPLY, ['rubber', 'mod_ore']), ['rubber']);
  assert.deepEqual(chosenResources(REPLY, ['mod_ore']), ['steel']);
});

test('over time, supply and demand break where the recording breaks', () => {
  const { supply, demand } = supplyDemandRuns(REPLY.series[0], 5);
  assert.deepEqual(supply.map(run => [run.firstHour, run.value, run.breakBefore]), [[100, 301, true], [124, 320, false]]);
  assert.deepEqual(demand.map(run => run.value), [105, 34]);
});

test('a chip toggles only its own resource and keeps resources this campaign lacks', () => {
  // Nothing stored: the automatic choice shown becomes explicit, plus the click.
  assert.deepEqual(toggleChosen(REPLY, [], ['steel'], 'rubber', 8), { value: 'steel rubber' });
  // Another campaign's modded resource stays saved through clicks here.
  assert.deepEqual(toggleChosen(REPLY, ['mod_ore', 'steel'], ['steel'], 'rubber', 8), { value: 'mod_ore steel rubber' });
  assert.deepEqual(toggleChosen(REPLY, ['mod_ore', 'steel', 'rubber'], ['steel', 'rubber'], 'steel', 8), { value: 'mod_ore rubber' });
  // The last resource this campaign shows stays chosen, and its chip says so.
  assert.deepEqual(toggleChosen(REPLY, ['mod_ore', 'steel'], ['steel'], 'steel', 8), { reason: 'One resource always stays chosen' });
  assert.deepEqual(toggleChosen(REPLY, [], ['steel'], 'steel', 8), { reason: 'One resource always stays chosen' });
  // The cap counts saved resources this campaign lacks, as the settings panel does.
  assert.deepEqual(toggleChosen(REPLY, ['mod_ore', 'steel'], ['steel'], 'rubber', 2), { reason: 'At most 2 can be chosen' });
});

test('an automatic resource that cannot be stored never blocks choosing another', () => {
  const long = 'un,obtainium';
  const modded: ResourceBalanceReply = { ...REPLY, resources: [{ id: 1, token: long, ordinal: 1, name: 'Unobtainium', icon: null }, ...REPLY.resources],
    series: [{ id: 1, runs: [pools(100, 130, { producedRaw: 5000 })] }, ...REPLY.series] };
  // It is the largest, so it is shown until something is chosen.
  assert.deepEqual(chosenResources(modded, []), [long]);
  // Choosing another stores only that one; the automatic one is not needed.
  assert.deepEqual(toggleChosen(modded, [], [long], 'steel', 8), { value: 'steel' });
  assert.deepEqual(toggleChosen(modded, [], [long], 'rubber', 8), { value: 'rubber' });
  // Its own chip cannot act, and says why rather than doing nothing.
  assert.deepEqual(toggleChosen(modded, [], [long], long, 8), { reason: UNSTORABLE });
  assert.deepEqual(toggleChosen(modded, ['steel'], ['steel'], long, 8), { reason: UNSTORABLE });
});

test('resource choices exceed eight within shared settings bounds', () => {
  const field = OPTIONS.find(field => field.key === 'resources')!;
  assert.ok(field.kind === 'content-set');
  const tokens = Array.from({ length: 12 }, (_, index) => `mod_resource_${index}`);
  const modded: ResourceBalanceReply = { ...REPLY, resources: tokens.map((token, id) => ({ id, token, ordinal: 1, name: null, icon: null })),
    series: tokens.map((_, id) => ({ id, runs: [STEEL] })) };
  const value = tokens.join(' ');
  assert.deepEqual(contentSet(resolveOptions(OPTIONS, parseStoredOptions({ resources: value })!).resources), tokens);
  assert.equal(chosenLines(modded, tokens).length, tokens.length);
  assert.deepEqual(toggleChosen(modded, tokens.slice(0, -1), tokens.slice(0, -1), tokens.at(-1)!, Number.MAX_SAFE_INTEGER), { value });
});

test('long deal lists keep their largest deals, sum the rest and always show the totals', () => {
  const exports = Array.from({ length: 50 }, (_, index) => trade(`T${String(index).padStart(2, '0')}`, 'export', 50 - index));
  const reserve = exports.reduce((sum, item) => sum + Number(item.deliveredRaw) / U, 0);
  const balance = balanceOf(pools(100, 123, { producedRaw: 2000, toExportRaw: reserve, exportedRaw: reserve + 5, availableRaw: 2000 - reserve,
    afterProjectsRaw: 2000 - reserve, productionRaw: -30 }));
  const rows = breakdownRows(balance, { imports: [], exports }, 5, tag => ({ name: tag }), 3);
  const labels = rows.map(row => row.label);
  assert.deepEqual(rows.filter(row => row.indent).map(row => [row.label, row.value]), [
    ['T00 · 1 civilian factory', '50'], ['T01 · 1 civilian factory', '49'], ['T02 · 1 civilian factory', '48'],
    // 47 + 46 + … + 1 = 1,128 in the other 47 deals; the remainder still counts every deal.
    ['47 more deals', '1,128'], ['Not from a listed deal', '+5'], ['Bought beyond the reserve', '5']]);
  assert.deepEqual(labels.slice(-3), ['Available', 'Needed by production lines', 'Surplus']);
  // One extra deal is named as one.
  assert.equal(breakdownRows(balance, { imports: [], exports: exports.slice(0, 2) }, 5, tag => ({ name: tag }), 1)
    .find(row => row.label.endsWith('more deal'))?.label, '1 more deal');
});

test('the keyboard stops where a chosen resource’s pools or deals change', () => {
  // Steel's runs start at 100 and 124; its deals run 100 to 123, so they end before 124;
  // the pools and the deal table end at 130, so 131 is not recorded.
  assert.deepEqual(changeHours(REPLY, [5]), [100, 124, 131]);
  const renewed: ResourceBalanceReply = { ...REPLY, trades: [...REPLY.trades, { ...trade('SWE', 'import', 3), firstHour: 110, lastHour: 115 }] };
  assert.deepEqual(changeHours(renewed, [5]), [100, 110, 116, 124, 131]);
  // Rubber's run and its one deal both end at 123; nothing shows for rubber at 131.
  assert.deepEqual(changeHours(REPLY, [7]), [100, 124]);
  assert.deepEqual(changeHours(REPLY, []), []);
});

test('the keyboard stops where the pools or the deal table stop being recorded, with no deal rows', () => {
  // Steel is not recorded from 110 to 119; the deal table is missing at 105 and 106.
  const gaps: ResourceBalanceReply = { ...REPLY, trades: [], dealHours: [{ firstHour: 100, lastHour: 104 }, { firstHour: 107, lastHour: 130 }],
    series: [{ id: 5, runs: [pools(100, 109, { producedRaw: 1 }), pools(120, 130, { producedRaw: 2 })] }, { id: 7, runs: [] }] };
  assert.deepEqual(changeHours(gaps, [5]), [100, 105, 107, 110, 120, 131]);
  // A deal table change where no chosen pools show changes nothing shown.
  const late: ResourceBalanceReply = { ...gaps, dealHours: [{ firstHour: 100, lastHour: 112 }, { firstHour: 115, lastHour: 125 }] };
  assert.deepEqual(changeHours(late, [5]), [100, 110, 120, 126, 131]);
  // Rubber has no pools, so no stops.
  assert.deepEqual(changeHours(gaps, [7]), []);
  assert.deepEqual(changeHours(gaps, []), []);
});

test('tooltip partners belong to the shown resource and follow the displayed deal order', () => {
  const tip = resourceTooltip({ title: 'Steel' }, balanceOf(STEEL), tradesAt(tradeIndex(REPLY), 5, 110), 5, tag => ({ name: tag }), 1200);
  assert.deepEqual(tooltipPartners(tip), ['SPR', 'SAU']);
  assert.deepEqual(tooltipPartners(null), []);
});

test('a date range cuts the series through the kit, so supply and demand stay inside it', () => {
  const series = REPLY.series[0];
  const cut = { id: series.id, runs: clipToSpans(series.runs, [{ firstHour: 110, lastHour: 126 }]) };
  assert.deepEqual(cut.runs.map(run => [run.firstHour, run.lastHour, run.startsSegment]), [[110, 123, false], [124, 126, false]]);
  assert.deepEqual(supplyDemandRuns(cut, 5).supply.map(run => [run.firstHour, run.lastHour, run.breakBefore]), [[110, 123, false], [124, 126, false]]);
  assert.deepEqual(supplyDemandRuns(cut, 5).demand.map(run => [run.firstHour, run.lastHour]), [[110, 123], [124, 126]]);
});
test('deals follow their own coverage: unknown where the deal table was missing, even while the pools go on', () => {
  // The review's case: the deal table was missing from hour 110 to 123, so the deal
  // runs end at 109 while the pools go on.
  const partial: ResourceBalanceReply = { ...REPLY, dealHours: [{ firstHour: 100, lastHour: 109 }, { firstHour: 124, lastHour: 130 }],
    trades: REPLY.trades.map(item => ({ ...item, lastHour: 109 })) };
  assert.equal(dealsKnown(partial, 105), true);
  assert.equal(dealsKnown(partial, 110), false);
  assert.equal(dealsKnown(partial, 124), true);
  assert.equal(dealsKnown(partial, 99), false);
  // At 115 the pools still show, but the whole import is not a remainder: the deals are unknown.
  const hour = 115;
  const run = runAt(partial.series[1].runs, hour)!;
  const shown = breakdownRows({ ...balanceOf(run), imported: 200000n }, dealsKnown(partial, hour) ? tradesAt(tradeIndex(partial), 7, hour) : null, 5, tag => ({ name: tag }));
  assert.ok(shown.some(row => row.label === 'Trade partners not recorded at this hour'));
  assert.ok(!shown.some(row => row.label === 'Not from a listed deal'));
  const rows = breakdownRows(balanceOf(STEEL), null, 5, tag => ({ name: tag }));
  assert.deepEqual(rows.filter(row => row.indent).map(row => row.label), ['Trade partners not recorded at this hour', 'Set aside, not bought']);
  assert.ok(!rows.some(row => row.label === 'Not from a listed deal'));
  // Nothing imported: nothing to say about import partners either.
  assert.equal(rows.filter(row => row.label.startsWith('Trade partners')).length, 1);
});

test('a repeated token is another resource: numbered, and chosen with its token', () => {
  const repeated: ResourceBalanceReply = { ...REPLY,
    resources: [...REPLY.resources, { id: 9, token: 'steel', ordinal: 2, name: 'Steel', icon: null }],
    series: [...REPLY.series, { id: 9, runs: [pools(100, 130, { producedRaw: 7 })] }] };
  assert.equal(validateResourceBalanceReply(repeated), repeated);
  assert.deepEqual(repeated.resources.map(resourceName), ['Steel', 'Resource name unavailable', 'Steel (2)']);
  assert.deepEqual(chosenLines(repeated, ['steel']).map(item => item.id), [5, 9]);
  assert.deepEqual(chosenLines(repeated, ['rubber', 'steel']).map(item => item.id), [7, 5, 9]);
});

test('the limit counts the lines drawn, and a repeated token counts every declaration', () => {
  const repeated: ResourceBalanceReply = { ...REPLY,
    resources: [...REPLY.resources, { id: 9, token: 'steel', ordinal: 2, name: 'Steel', icon: null }],
    series: [...REPLY.series, { id: 9, runs: [pools(100, 130, { producedRaw: 7 })] }] };
  // Rubber plus both steels are three lines, past a limit of two.
  assert.deepEqual(toggleChosen(repeated, ['rubber'], ['rubber'], 'steel', 2), { reason: 'At most 2 lines can be drawn; Steel adds 2' });
  assert.deepEqual(toggleChosen(repeated, ['rubber'], ['rubber'], 'steel', 3), { value: 'rubber steel' });
  // Removing is always allowed while something stays shown.
  assert.deepEqual(toggleChosen(repeated, ['rubber', 'steel'], ['rubber', 'steel'], 'steel', 2), { value: 'rubber' });
});

test('the reply’s deal index is built once and answers at a run’s edges', () => {
  assert.equal(tradeIndex(REPLY), tradeIndex(REPLY));
  // Hours at a run's edges: its first hour, its last, and the hour after.
  assert.deepEqual(tradesAt(tradeIndex(REPLY), 5, 100).exports.map(trade => trade.partner), ['SPR', 'SAU']);
  assert.deepEqual(tradesAt(tradeIndex(REPLY), 5, 123).exports.map(trade => trade.partner), ['SPR', 'SAU']);
  assert.deepEqual(tradesAt(tradeIndex(REPLY), 5, 124), { imports: [], exports: [] });
});

test('five thousand hover partners request names and flags only for the fitted rows', () => {
  const reply = { ...REPLY, trades: Array.from({ length: 5000 }, (_, index) =>
    trade(`C${index}`, index % 2 ? 'export' : 'import', index === 4999 ? -1 : 5000 - index, index % 5 ? 5 : 7)) };
  const deals = tradesAt(tradeIndex(reply), 5, 110);
  const sum = (list: readonly TradeRun[]) => list.reduce((total, item) => total + BigInt(item.deliveredRaw), 0n);
  const imported = sum(deals.imports), exported = sum(deals.exports);
  const balance = { ...balanceOf(STEEL), imported, exported, exportReserve: exported };
  for (const height of [180, 300, 600, 1200]) {
    const tip = resourceTooltip(HEAD, balance, deals, 5, tag => ({ name: tag }), height);
    const tags = tooltipPartners(tip);
    const rows = tip.rows.filter(row => row.label.includes('civilian factory'));
    assert.deepEqual(tags, rows.map(row => row.label.split(' · ')[0]));
    assert.ok(tags.length <= MAX_DEALS * 2);
    assert.ok(tags.every(tag => Number(tag.slice(1)) % 5 !== 0), 'hidden resource partners are not wanted');
    for (const list of [deals.imports, deals.exports]) {
      const shown = tags.filter(tag => list.some(trade => trade.partner === tag));
      assert.deepEqual(shown, list.slice(0, shown.length).map(trade => trade.partner));
      assert.ok(shown.length <= MAX_DEALS);
    }
    const wanted = tags.map(tag => ({ tag, hour: 110 }));
    // Both mounted label hooks use this request planner (names and small flags).
    for (const chunk of [64, 32]) {
      const plan = planRequests(new HourStore<string>(1), wanted, 1, chunk);
      assert.equal(plan.flatMap(([, requested]) => requested).length, rows.length);
      assert.ok(plan.every(([hour]) => hour === 110));
    }
    assert.equal(tip.rows.at(-1)!.value, '+196');
  }
  const full = resourceTooltip(HEAD, balance, deals, 5, tag => ({ name: tag }), 1200);
  assert.equal(full.rows.find(row => row.label === 'Imported')!.value, `+${formatAmount(imported, 5)}`);
  const remainder = full.rows.filter(row => row.label.endsWith('more deals')).map(row => row.value);
  assert.deepEqual(remainder, [formatAmount(sum(deals.imports.slice(MAX_DEALS)), 5), formatAmount(sum(deals.exports.slice(MAX_DEALS)), 5)]);
  const lines = reply.resources.map(resource => ({ resource, color: 'c', balance,
    deals: tradesAt(tradeIndex(reply), resource.id, 110) }));
  const time = overTimeTooltip('t', lines, 5, tag => ({ name: tag }), 1200);
  assert.deepEqual(tooltipPartners(time), time.rows.filter(row => row.label.includes('civilian factory')).map(row => row.label.split(' · ')[0]));
  assert.ok(tooltipPartners(time).length <= MAX_DEALS * 2 * FULL_BREAKDOWN_MAX);
  assert.deepEqual(tooltipPartners(overTimeTooltip('t', lines, 5, tag => ({ name: tag }), 180)), []);
});

/** A resource with `count` deals each way, the pools matching them. */
function busy(count: number) {
  const exports = Array.from({ length: count }, (_, index) => trade(`E${String(index).padStart(2, '0')}`, 'export', 60 - index));
  const imports = Array.from({ length: count }, (_, index) => trade(`I${String(index).padStart(2, '0')}`, 'import', 30 - index));
  const sum = (list: TradeRun[]) => list.reduce((total, item) => total + Number(item.deliveredRaw) / U, 0);
  const balance = balanceOf(pools(100, 123, { producedRaw: 2000, importedRaw: sum(imports), toExportRaw: sum(exports) + 3, exportedRaw: sum(exports),
    availableRaw: 2000 + sum(imports) - sum(exports) - 3, afterProjectsRaw: 2000 + sum(imports) - sum(exports) - 3, productionRaw: -40 }));
  return { balance, deals: { imports, exports } };
}
const HEAD = { title: 'Steel', icon: 'data:image/png;base64,', subtitle: 'Per day · 1 Jan 1936' };

test('a resource’s tooltip fits any window with its surplus or deficit shown', () => {
  // The review's case: a 300px window and many deals each way.
  for (const height of [180, 240, 300, 360, 450, 600, 800, 1200]) {
    for (const count of [0, 1, 3, 8, 20]) {
      const { balance, deals } = busy(count);
      const tip = resourceTooltip(HEAD, balance, deals, 5, tag => ({ name: tag }), height);
      assert.ok(tooltipHeight(tip) <= tooltipRoom(height), `${count} deals at ${height}: ${tooltipHeight(tip)}`);
      assert.equal(tip.rows.at(-1)!.label, 'Surplus', `${count} deals at ${height}`);
      // A screen reader gets the full breakdown, every deal up to the limit, whatever fits.
      const spoken = tooltipText(tip);
      for (const deal of deals.exports.slice(0, MAX_DEALS)) assert.ok(spoken.includes(`${deal.partner} · 1 civilian factory`), `${deal.partner} at ${height}`);
    }
  }
  const { balance, deals } = busy(20);
  // Tall enough, it lists every deal it may; at 300px the largest deal each way and the
  // rest summed; at 280px it keeps supply, demand and the net.
  assert.equal(resourceTooltip(HEAD, balance, deals, 5, tag => ({ name: tag }), 1200).rows.filter(row => row.label.includes('civilian factory')).length, 2 * MAX_DEALS);
  assert.deepEqual(resourceTooltip(HEAD, balance, deals, 5, tag => ({ name: tag }), 300).rows.filter(row => row.indent).map(row => row.label),
    ['I00 · 1 civilian factory', '19 more deals', 'E00 · 1 civilian factory', '19 more deals', 'Set aside, not bought']);
  assert.deepEqual(resourceTooltip(HEAD, balance, deals, 5, tag => ({ name: tag }), 280).rows.map(row => row.label),
    ['Supply', 'Demand', 'Surplus']);
});

test('an over-time tooltip fits the window with every chosen resource’s net', () => {
  const { balance, deals } = busy(6);
  const line = (index: number) => ({ resource: { id: index, token: `r${index}`, ordinal: 1, name: `R${index}`, icon: null }, color: 'c', balance, deals });
  for (const height of [200, 300, 450, 600, 800, 1200]) {
    for (let count = 1; count <= 8; count++) {
      const tip = overTimeTooltip('1 Jan 1936 · per day', Array.from({ length: count }, (_, index) => line(index)), 5, tag => ({ name: tag }), height);
      assert.ok(tooltipHeight(tip) <= tooltipRoom(height), `${count} at ${height}`);
      // Every resource is named, or counted in the last row.
      const named = tip.rows.filter(row => /^R\d$/.test(row.label)).length;
      const counted = Number(/^(\d+) more resources?/.exec(tip.rows.at(-1)!.label)?.[1] ?? 0);
      assert.equal(named + counted, count, `${count} at ${height}`);
      // Breakdowns are read out for at most two resources; more get their summaries.
      assert.equal(tooltipText(tip).includes('E00 · 1 civilian factory'), count <= FULL_BREAKDOWN_MAX, `${count} at ${height}`);
      assert.equal(tooltipText(tip).split('Surplus').length - 1, count, `${count} at ${height}`);
    }
  }
  // Two resources in a tall window: both breakdowns, as many deals as fit.
  const two = overTimeTooltip('t', [line(1), line(2)], 5, tag => ({ name: tag }), 1200);
  assert.equal(two.rows.filter(row => row.label === 'Surplus').length, 2);
  assert.equal(two.spoken, undefined);
});

test('malformed replies are refused', () => {
  assert.equal(validateResourceBalanceReply(REPLY), REPLY);
  const broken = (change: Partial<ResourceBalanceReply>) => () => validateResourceBalanceReply({ ...REPLY, ...change });
  assert.throws(broken({ series: [REPLY.series[1], REPLY.series[0]] }));
  assert.throws(broken({ series: [{ id: 5, runs: [STEEL, STEEL] }, REPLY.series[1]] }));
  assert.throws(broken({ trades: [{ ...REPLY.trades[0], resource: 99 }] }));
  assert.throws(broken({ resources: [{ ...REPLY.resources[0], icon: 'javascript:alert(1)' }, REPLY.resources[1]] }));
  assert.throws(broken({ dealHours: [{ firstHour: 124, lastHour: 130 }, { firstHour: 100, lastHour: 110 }] }));
  assert.throws(broken({ resources: [REPLY.resources[1], REPLY.resources[0]], series: [REPLY.series[1], REPLY.series[0]] }));
});
