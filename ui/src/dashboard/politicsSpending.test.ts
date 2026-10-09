import test from 'node:test';
import assert from 'node:assert/strict';
import type { LedgerSpend, LedgerSpendItem, PoliticsLedgerReply } from '../generated/dashboard';
import { categoryChoices, commandHeldAt, commandHeldTooltip, spendingAt, spendingGaps, spendingGroups, spendingItems, spendingRows, spendingTotals, spendText, validatePoliticsSpending } from './politicsSpending.ts';
import { setToken } from './options.ts';
import { groupTree } from '../charts/sunburst/layout.ts';
import { fitTooltip, tooltipText } from '../charts/tooltip.ts';
import { formatDateTime } from './calendar.ts';
import type { SpendingSelection } from './politicsSpending';
import { definition as politicalSpending } from '../widgets/political-power-spending/definition.ts';
import { definition as commandSpending } from '../widgets/command-power-spending/definition.ts';
import { definition as spendingBreakdown } from '../widgets/political-power-spending-breakdown/definition.ts';
import { POLITICS_SPENDING_OPTIONS } from './politicsSpendingOptions.ts';
import { contentSet, parseStoredOptions, resolveOptions, toggleContentSet } from './options.ts';

const ICON = 'data:image/png;base64,YQ==';
const item = (id = 'mod/law'): LedgerSpendItem => ({ id, groupId: 'law:mod-law', name: 'A modded law', action: 'A modded law', icon: ICON,
  tooltipToken: null, character: null, targetTag: null, targetName: null, unknown: null });
const spend = (overrides: Partial<LedgerSpend> = {}): LedgerSpend => ({ id: 'charge:1', hour: 10, readHour: 12, sequence: '1', ordinal: 0,
  countryTag: 'mod country', countryName: 'Recorded republic', currency: 'Political', category: 'law/mod', itemId: 'mod/law',
  chargeRaw: '123456789012345678901', realizedDeltaRaw: '-2500000', covered: true, occurrences: 1,
  changes: [{ readHour: 12, chargeRaw: '123456789012345678901', realizedDeltaRaw: '-2500000', occurrences: 1, covered: true }], ...overrides });
const reply = (overrides: Partial<PoliticsLedgerReply> = {}): PoliticsLedgerReply => ({ branchId: 'branch', tag: 'mod country', decimals: 5,
  mutations: [], governments: [], daily: [], dailyCosts: [], coverage: [], warnings: [], spending: [spend()], spendItems: [item()],
  spendCategories: [{ id: 'law/mod', name: 'Mod laws', identified: true }], ideaTooltips: null, commandHolds: [],
  spendingCoverage: [{ readHour: 0, sinceHour: 0, throughHour: 0, political: true, command: true, reason: null }], ...overrides });

test('country comparison and category settings retain every recorded choice', () => {
  for (const definition of [politicalSpending, commandSpending, spendingBreakdown]) assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const tokens = Array.from({ length: 50 }, (_, index) => setToken(`mod category ${index}`));
  let stored = '';
  for (const token of tokens) stored = toggleContentSet(stored, token, Number.MAX_SAFE_INTEGER)!;
  const parsed = parseStoredOptions({ categories: stored });
  assert.ok(parsed);
  assert.deepEqual(contentSet(resolveOptions(POLITICS_SPENDING_OPTIONS, parsed).categories), tokens);
});

test('mid-day membership and a date range select upkeep by native payment hour, never drain hour', () => {
  const data = reply({ spending: [spend({ currency: 'Command', changes: [
    { hour: 10, readHour: 12, chargeRaw: '1', realizedDeltaRaw: '-1', occurrences: 1, covered: true },
    { hour: 15, readHour: 18, chargeRaw: '2', realizedDeltaRaw: '-2', occurrences: 1, covered: true },
    { hour: 21, readHour: 22, chargeRaw: '3', realizedDeltaRaw: '-3', occurrences: 1, covered: true },
  ] })] });
  const selection: SpendingSelection = { matches: (tag, hour) => tag === data.tag && hour >= 14 && hour <= 19,
    membership: new Map([[data.tag, [{ firstHour: 14, lastHour: 19 }]]]), unobserved: [] };
  assert.deepEqual(spendingAt([data], 'Command', 17, 14, selection), []);
  const [at] = spendingAt([data], 'Command', 22, 14, selection);
  assert.equal(at.charge, 2n);
  assert.equal(at.spend.hour, 15);
  assert.equal(at.spend.countryName, null);
  assert.equal(at.readHour, 18);
  assert.equal(at.occurrences, 1);
  assert.equal(spendingAt([data], 'Command', 22, 14)[0].charge, 5n);
  assert.equal(groupTree(spendingItems([data], 'Command', { from: 14, hour: 22, end: 22 }, selection), { id: 'all', label: 'Costs' }).exact, 2n);
});

test('an older aggregate without native contribution dates refuses an uncertain membership subtotal', () => {
  const data = reply({ spending: [spend({ changes: [
    { readHour: 12, chargeRaw: '1', realizedDeltaRaw: '-1', occurrences: 1, covered: true },
    { readHour: 18, chargeRaw: '2', realizedDeltaRaw: '-2', occurrences: 1, covered: true },
  ] })] });
  const selection: SpendingSelection = { matches: (_, hour) => hour >= 14,
    membership: new Map([[data.tag, [{ firstHour: 14, lastHour: 30 }]]]), unobserved: [] };
  const [at] = spendingAt([data], 'Political', 20, null, selection);
  assert.equal(at.charge, null);
  assert.equal(at.recordedCharge, 0n);
  assert.equal(at.covered, false);
  assert.match(at.item.unknown!, /Native payment hours/);
  assert.equal(spendingTotals([at], []).total, null);
});

test('manual native-holder gaps keep current command holdings unknown while known zero and empty groups remain distinct', () => {
  const data = reply({ commandHolds: [{ readHour: 10, throughHour: 20, totalRaw: '0', holders: [], reason: null }] });
  const selection: SpendingSelection = { matches: (tag, hour) => tag === data.tag && hour >= 10 && hour <= 14,
    membership: new Map([[data.tag, [{ firstHour: 10, lastHour: 14 }]]]), unobserved: [], requireRecordedHolder: true };
  assert.equal(commandHeldAt([data], 14, selection).total, 0n, 'a complete selected native holder can certify zero');
  const gap = commandHeldAt([data], 15, selection);
  assert.equal(gap.total, null); assert.deepEqual(gap.countries, []);
  assert.match(gap.reason!, /No selected native country holder was recorded/);
  assert.equal(commandHeldAt([data], 15, { ...selection, requireRecordedHolder: false }).total, 0n,
    'a known empty group has no expected holder and may sum to zero');
  const missing = commandHeldAt([reply({ commandHolds: [] })], 14, selection);
  assert.equal(missing.total, null); assert.equal(missing.countries.length, 1, 'existing missing hold operands remain unknown');
  const aggregate = reply({ spending: [spend({ changes: [
    { readHour: 12, chargeRaw: '1', realizedDeltaRaw: '-1', occurrences: 1, covered: true },
    { readHour: 18, chargeRaw: '2', realizedDeltaRaw: '-2', occurrences: 1, covered: true },
  ] })] });
  const [unknown] = spendingAt([aggregate], 'Political', 20, null, selection);
  assert.equal(unknown.occurrences, 2); assert.equal(unknown.charge, null); assert.equal(unknown.covered, false);
  assert.match(unknown.item.unknown!, /Native payment hours/);
  assert.equal(spendingTotals([unknown], []).total, null, 'undated contributions remain explicit unknown rows');
});

test('group coverage clips to membership and an unread membership hour keeps spending and holds unknown', () => {
  const data = reply({ spending: [], spendingCoverage: [{ readHour: 0, sinceHour: 0, throughHour: 30, political: false, command: false, reason: 'Native stream missing' }] });
  const selection: SpendingSelection = { matches: (_, hour) => hour >= 14 && hour <= 19 && hour !== 16,
    membership: new Map([[data.tag, [{ firstHour: 14, lastHour: 19 }]]]), unobserved: [{ firstHour: 16, lastHour: 16 }] };
  const gaps = spendingGaps([data], 'Political', { from: null, hour: 30 }, selection);
  assert.deepEqual(gaps.map(gap => [gap.from, gap.through]), [[14, 19], [16, 16]]);
  assert.equal(spendingTotals([], gaps).total, null);
  const held = commandHeldAt([data], 16, selection);
  assert.equal(held.total, null);
  assert.match(commandHeldTooltip(held).rows[0].label, /Group membership/);
  const complete = reply({ spending: [], spendingCoverage: [{ readHour: 0, sinceHour: 0, throughHour: 30, political: true, command: true, reason: null }] });
  assert.equal(spendingGaps([complete], 'Political', { from: null, hour: 30 }, selection).length, 1);
});

test('all spending views preserve exact native charges and mod content, without early reads', () => {
  const data = validatePoliticsSpending(reply());
  assert.deepEqual(spendingAt([data], 'Political', 11), []);
  const [at] = spendingAt([data], 'Political', 12);
  assert.equal(at.spend.hour, 10);
  assert.equal(at.readHour, 12);
  assert.equal(at.charge, 123456789012345678901n);
  assert.equal(at.realized, -2500000n);
  assert.equal(at.item.name, 'A modded law');
  assert.equal(at.item.icon, ICON);
  assert.equal(spendText(at.charge), '1234567890123456.78901');
  const tree = groupTree(spendingItems([data], 'Political', { from: null, hour: 12, end: 12 }), { id: 'all', label: 'Spending' });
  assert.equal(tree.exact, at.charge);
  assert.equal(tree.children[0].label, 'Mod laws');
});

test('a renamed target retains each action hour’s recorded name', () => {
  const old = { ...item('old'), name: 'Old target', action: 'Improve relations: Old target', targetTag: 'target', targetName: 'Old target' };
  const renamed = { ...item('renamed'), name: 'New target', action: 'Improve relations: New target', targetTag: 'target', targetName: 'New target' };
  const data = reply({ spendItems: [old, renamed], spending: [spend({ itemId: old.id }), spend({ id: 'second', hour: 30, readHour: 31,
    itemId: renamed.id, changes: [{ readHour: 31, chargeRaw: '100000', realizedDeltaRaw: '-100000', occurrences: 1, covered: true }] })] });
  assert.equal(spendingAt([data], 'Political', 12)[0].item.targetName, 'Old target');
  assert.deepEqual(spendingAt([data], 'Political', 31).map(at => at.item.action), ['Improve relations: Old target', 'Improve relations: New target']);
});

test('modded law, advisor portrait and decision labels and art survive the shared projection', () => {
  const law = item('law-context');
  const advisor = { ...item('advisor-context'), groupId: 'advisor:mod', name: 'Mod advisor', action: 'Hired Mod advisor', character: 42 };
  const decision = { ...item('decision-context'), groupId: 'decision:mod', name: 'Mod decision', action: 'Mod decision' };
  const data = validatePoliticsSpending(reply({ spendItems: [law, advisor, decision], spendCategories: [
    { id: 'law/mod', name: 'Mod laws', identified: true }, { id: 'advisor', name: 'Advisors', identified: true }, { id: 'decision', name: 'Decisions', identified: true },
  ], spending: [spend({ itemId: law.id }), spend({ id: 'advisor-action', category: 'advisor', itemId: advisor.id }),
    spend({ id: 'decision-action', category: 'decision', itemId: decision.id })] }));
  const at = spendingAt([data], 'Political', 12);
  assert.deepEqual(at.map(row => [row.item.action, row.item.icon, row.category]), [
    ['A modded law', ICON, 'Mod laws'], ['Hired Mod advisor', ICON, 'Advisors'], ['Mod decision', ICON, 'Decisions'],
  ]);
});

test('upkeep is one daily row with read-dated increments and coverage', () => {
  const data = reply({ spending: [spend({ id: 'upkeep-day', currency: 'Command', category: 'upkeep', chargeRaw: '400000',
    realizedDeltaRaw: '-400000', covered: false, occurrences: 4, changes: [
      { readHour: 12, chargeRaw: '100000', realizedDeltaRaw: '-100000', occurrences: 1, covered: true },
      { readHour: 15, chargeRaw: '300000', realizedDeltaRaw: '-300000', occurrences: 3, covered: false },
    ] })], spendCategories: [{ id: 'upkeep', name: 'Upkeep', identified: true }] });
  const earlier = spendingAt([data], 'Command', 12);
  assert.equal(earlier.length, 1);
  assert.equal(earlier[0].charge, 100000n);
  assert.equal(earlier[0].occurrences, 1);
  assert.equal(earlier[0].covered, true);
  assert.equal(spendingTotals(earlier, []).total, 100000n);
  const later = spendingAt([data], 'Command', 15);
  assert.equal(later.length, 1);
  assert.equal(later[0].charge, 400000n);
  assert.equal(later[0].occurrences, 4);
  assert.equal(later[0].covered, false);
  assert.equal(spendingTotals(later, []).total, null);
});

test('missing operands and coverage gaps are never zero and never Fill’s denominator', () => {
  const data = reply({ spending: [spend({ chargeRaw: null, covered: false,
    changes: [{ readHour: 12, chargeRaw: null, realizedDeltaRaw: '-200000', occurrences: 1, covered: false }] })],
  spendingCoverage: [{ readHour: 12, sinceHour: 10, throughHour: 12, political: false, command: true, reason: 'Dropped observations' },
    { readHour: 13, sinceHour: 12, throughHour: 13, political: false, command: true, reason: 'Dropped observations' }] });
  const gaps = spendingGaps([data], 'Political', { from: null, hour: 13 });
  assert.equal(gaps.length, 1);
  assert.equal(gaps[0].through, 13);
  const at = spendingAt([data], 'Political', 13);
  assert.equal(at[0].charge, null);
  assert.equal(spendingTotals(at, gaps).total, null);
  assert.equal(spendingTotals(at, gaps).categories.get(setToken('law/mod'))?.value, null);
  const items = spendingItems([data], 'Political', { from: null, hour: 13, end: 13 });
  const tree = groupTree(items, { id: 'all', label: 'Spending' });
  assert.equal(tree.valueUnknown, true);
  assert.equal(tree.finalUnknown, true);
  assert.equal(tree.children.find(child => child.key === 'coverage-gap')?.label, 'Hours not recorded');
  assert.equal(spendingRows(at, gaps, setToken('law/mod'), 'newest').length, 1, 'A category filter never hides gaps');
  const empty = reply({ spending: [], spendingCoverage: [] });
  assert.equal(spendingTotals([], spendingGaps([empty], 'Political', { from: null, hour: 12 })).total, null);
});

test('coverage gaps and unidentified spending have separate top-level sunburst paths', () => {
  const data = reply({ spendCategories: [{ id: 'unidentified', name: 'Not identified', identified: false }],
    spending: [spend({ category: 'unidentified' })], spendingCoverage: [{ readHour: 12, sinceHour: 12, throughHour: 12,
      political: false, command: true, reason: 'Hours missing' }] });
  const tree = groupTree(spendingItems([data], 'Political', { from: null, hour: 12, end: 12 }), { id: 'all', label: 'Spending' });
  assert.deepEqual(new Set(tree.children.map(child => child.label)), new Set(['Not identified', 'Hours not recorded']));
  assert.equal(tree.children.find(child => child.label === 'Hours not recorded')?.children.length, 0);
});

test('command power allocations are current holdings, separate from spending at every read', () => {
  const data = validatePoliticsSpending(reply({ spending: [], commandHolds: [
    { readHour: 10, throughHour: 10, totalRaw: '500000', holders: [{ id: 'leader:42', kind: 'General', name: 'Recorded leader', amountRaw: '500000' }], reason: null },
    { readHour: 11, throughHour: 11, totalRaw: '0', holders: [], reason: null },
    { readHour: 12, throughHour: 13, totalRaw: '800000', holders: [{ id: 'leader:42', kind: 'General', name: 'Renamed leader', amountRaw: '800000' }], reason: null },
    { readHour: 15, throughHour: 15, totalRaw: null, holders: [{ id: 'ability:3', kind: 'Ability', name: null, amountRaw: null }], reason: 'Allocation amount was not recorded' },
  ] }));
  for (const [hour, total] of [[9, null], [10, 500000n], [11, 0n], [12, 800000n], [13, 800000n], [14, null], [15, null]] as const) {
    const held = commandHeldAt([data], hour);
    assert.equal(held.total, total, `current holdings at ${hour}`);
    assert.equal(spendingTotals(spendingAt([data], 'Command', hour), []).total, 0n, `allocations add no spending at ${hour}`);
  }
  const tooltip = commandHeldTooltip(commandHeldAt([data], 12));
  assert.ok(tooltip.rows.some(row => row.label === 'Recorded through' && row.value === formatDateTime(12)), 'A later witness never leaks past the current moment');
  assert.ok(tooltip.rows.some(row => row.label === 'General: Renamed leader' && row.value === '8'));
  assert.ok(!tooltip.rows.some(row => row.label.includes('Recorded leader')));
  const continued = commandHeldTooltip(commandHeldAt([data], 13));
  assert.ok(continued.rows.some(row => row.label === 'Read' && row.value === formatDateTime(12)));
  assert.ok(continued.rows.some(row => row.label === 'Recorded through' && row.value === formatDateTime(13)));
  const unknown = commandHeldTooltip(commandHeldAt([data], 15));
  assert.ok(unknown.rows.some(row => row.label === 'Ability: Not identified' && row.value === 'Not known'));
});

test('5,000 country holdings sum exactly and one missing country keeps the complete figure unknown', () => {
  const data = Array.from({ length: 5000 }, (_, index) => validatePoliticsSpending(reply({ tag: `mod ${index}`, spending: [], commandHolds: [
    { readHour: 12, throughHour: 12, totalRaw: '100001', holders: [{ id: `raid:${index}`, kind: 'Raid', name: `Recorded raid ${index}`, amountRaw: '100001' }], reason: null },
  ] })));
  assert.equal(commandHeldAt(data, 12).total, 500005000n);
  data[0] = reply({ tag: 'mod 0', commandHolds: [] });
  const held = commandHeldAt(data, 12);
  assert.equal(held.total, null);
  assert.equal(held.countries[1].hold?.holders[0].name, 'Recorded raid 1');
  const full = commandHeldTooltip(held);
  const fitted = fitTooltip([full], 300, { keep: 1 });
  assert.equal(fitted.rows.at(-1)?.label, 'Total held');
  assert.ok(fitted.rows.some(row => row.label.includes('more rows')));
  assert.ok(tooltipText(fitted).includes('Raid: Recorded raid 4999'));
});

test('native signed allocation operands are displayed exactly without clamping or spending', () => {
  const data = validatePoliticsSpending(reply({ spending: [], commandHolds: [{ readHour: 12, throughHour: 12, totalRaw: '-250001',
    holders: [{ id: 'orders:7', kind: 'OrdersGroup', name: 'Recorded orders', amountRaw: '-250001' }], reason: null }] }));
  const held = commandHeldAt([data], 12);
  assert.equal(held.total, -250001n);
  assert.ok(commandHeldTooltip(held).rows.some(row => row.label === 'OrdersGroup: Recorded orders' && row.value === '-2.50001'));
  assert.equal(spendingTotals(spendingAt([data], 'Command', 12), []).total, 0n);
});

test('Daily deficit remains its own recorded category alongside real costs', () => {
  const data = validatePoliticsSpending(reply({ spending: [spend({ id: 'deficit', currency: 'Command', category: 'daily-deficit' }),
    spend({ id: 'ability', currency: 'Command', category: 'daily-costs' })], spendCategories: [
    { id: 'daily-deficit', name: 'Daily deficit', identified: true }, { id: 'daily-costs', name: 'Daily costs', identified: true },
  ] }));
  const choices = categoryChoices([data], 'Command');
  assert.deepEqual(choices.map(choice => choice.label), ['Daily deficit', 'Daily costs']);
  const rows = spendingRows(spendingAt([data], 'Command', 12), [], 'daily-costs', 'newest');
  assert.equal(rows.length, 1);
  assert.equal(rows[0].item.kind === 'spend' ? rows[0].item.at.category : '', 'Daily deficit');
});

test('the spending view needs no raw ledger tables and resolves repeated tooltip icons from one artwork entry', () => {
  const info = { token: 'dated-law', category: 'laws', slot: null, law: true, designer: false, effectsKnown: true,
    unknown: null, effects: [], research: [], targeted: [], equipment: [], traits: [], tooltips: [] };
  const full = reply({ spendItems: [{ ...item('first'), icon: null, tooltipToken: 'dated-law' }, { ...item('second'), icon: null, tooltipToken: 'dated-law' }],
    spending: [spend({ itemId: 'first' }), spend({ id: 'second-charge', itemId: 'second' })], ideaTooltips: { ideas: [info],
      artwork: [{ token: 'dated-law', name: 'Mod law', description: null, baseDurationDays: null, icon: ICON }], modifiers: [], texts: [] } });
  const { mutations: _mutations, governments: _governments, daily: _daily, dailyCosts: _dailyCosts, coverage: _coverage, ...view } = full;
  const data = validatePoliticsSpending(view);
  assert.equal(data.ideaTooltips?.artwork.length, 1);
  assert.deepEqual(data.spendItems.map(item => item.icon), [null, null]);
  assert.deepEqual(spendingAt([data], 'Political', 12).map(at => at.item.icon), [ICON, ICON]);
});

test('coverage uncertainty keeps known recorded charges while marking complete totals unknown', () => {
  const data = reply({ spending: [spend({ covered: false, changes: [{ readHour: 12, chargeRaw: '2500000', realizedDeltaRaw: '-2000000', occurrences: 1, covered: false }] })] });
  const [at] = spendingAt([data], 'Political', 12);
  assert.equal(at.charge, 2500000n);
  const [leaf] = spendingItems([data], 'Political', { from: null, hour: 12, end: 12 });
  assert.equal(leaf.exact, 2500000n);
  assert.equal(leaf.unknownValue, true);
});

test('compressed gap intervals never reveal a later end past the playhead', () => {
  const data = reply({ spendingCoverage: [
    { readHour: 12, sinceHour: 10, throughHour: 12, political: false, command: true, reason: 'Dropped observations' },
    { readHour: 12, sinceHour: 12, throughHour: 60, political: false, command: true, reason: 'Dropped observations' },
  ] });
  const gaps = spendingGaps([data], 'Political', { from: null, hour: 13 });
  assert.equal(gaps.length, 1);
  assert.equal(gaps[0].through, 13);
});

test('missing charge in a daily row retains every known contribution without completing the total', () => {
  const data = reply({ spending: [spend({ currency: 'Command', chargeRaw: null, realizedDeltaRaw: null, occurrences: 2,
    changes: [{ readHour: 12, chargeRaw: '250001', realizedDeltaRaw: '-250001', occurrences: 1, covered: true },
      { readHour: 13, chargeRaw: null, realizedDeltaRaw: null, occurrences: 1, covered: true }] })] });
  const [at] = spendingAt([data], 'Command', 13);
  assert.equal(at.charge, null);
  assert.equal(at.recordedCharge, 250001n);
  assert.equal(at.knownCharges, 1);
  assert.equal(at.missingCharges, 1);
  assert.equal(at.realized, null);
  assert.equal(spendingTotals([at], []).total, null);
  const [leaf] = spendingItems([data], 'Command', { from: null, hour: 13, end: 13 });
  assert.equal(leaf.exact, 250001n);
  assert.equal(leaf.unknownValue, true);
});

test('repeated charges for the same specific item form one sunburst leaf', () => {
  const data = reply({ spendItems: [item('first-context'), item('second-context')], spending: [spend({ id: 'first', itemId: 'first-context' }),
    spend({ id: 'second', itemId: 'second-context' })] });
  const tree = groupTree(spendingItems([data], 'Political', { from: null, hour: 12, end: 12 }), { id: 'all', label: 'Spending' });
  assert.equal(tree.children[0].children.length, 1);
  assert.equal(tree.children[0].children[0].exact, 123456789012345678901n * 2n);
  assert.equal(tree.children[0].children[0].count, 2);
});

test('grouped item tooltip keeps all known charges and dates effects to the latest matched action', () => {
  const data = reply({ spending: [spend({ id: 'first', sequence: '9', changes: [{ readHour: 12, chargeRaw: '111111', realizedDeltaRaw: '-1', occurrences: 1, covered: true }] }),
    spend({ id: 'second', hour: 13, readHour: 14, sequence: '10', changes: [{ readHour: 14, chargeRaw: '222222', realizedDeltaRaw: '-1', occurrences: 1, covered: true }] }),
    spend({ id: 'missing', hour: 15, readHour: 16, sequence: '11', changes: [{ readHour: 16, chargeRaw: null, realizedDeltaRaw: null, occurrences: 1, covered: true }] })] });
  const before = spendingGroups(spendingAt([data], 'Political', 14)).get('law:mod-law')!;
  assert.equal(before.latest.spend.id, 'second');
  assert.equal(before.recordedCharge, 333333n);
  const after = spendingGroups(spendingAt([data], 'Political', 16)).get('law:mod-law')!;
  assert.equal(after.latest.spend.id, 'missing');
  assert.equal(after.recordedCharge, 333333n);
  assert.equal(after.knownCharges, 2);
  assert.equal(after.missingCharges, 1);
});

test('grouped item tooltip uses the latest native ordinal within the same action sequence', () => {
  const data = reply({ spending: [spend({ id: 'later', ordinal: 2 }), spend({ id: 'earlier', ordinal: 1 })] });
  const group = spendingGroups(spendingAt([data], 'Political', 12)).get('law:mod-law')!;
  assert.equal(group.latest.spend.id, 'later');
});

test('running category totals distinguish a missing charge from no spending without relying on a gap', () => {
  const data = reply({ spending: [spend({ changes: [{ readHour: 12, chargeRaw: null, realizedDeltaRaw: null, occurrences: 1, covered: true }] })] });
  const totals = spendingTotals(spendingAt([data], 'Political', 12), []);
  assert.equal(totals.categoryTotal(setToken('law/mod')), null);
  assert.equal(spendText(totals.categoryTotal(setToken('law/mod'))), 'Not known');
  assert.equal(totals.categoryTotal('unseen-kind'), 0n);
  const gaps = spendingGaps([reply({ spendingCoverage: [{ readHour: 12, sinceHour: 12, throughHour: 12, political: false, command: false, reason: 'Missing data' }] })], 'Political', { from: null, hour: 12 });
  assert.equal(spendingTotals([], gaps).categoryTotal('unseen-kind'), null);
});

test('mockup category chips hide chosen kinds and leave all-category totals unchanged', () => {
  const data = reply({ spending: [spend(), spend({ id: 'second', category: 'advisor' })],
    spendCategories: [{ id: 'law/mod', name: 'Mod laws', identified: true }, { id: 'advisor', name: 'Advisors', identified: true }] });
  const at = spendingAt([data], 'Political', 12);
  const totals = spendingTotals(at, []);
  assert.equal(spendingRows(at, [], '', 'newest').length, 2);
  assert.equal(spendingRows(at, [], 'advisor', 'newest').length, 1);
  assert.equal(spendingRows(at, [], 'advisor', 'newest', 'include')[0].item.kind, 'spend');
  assert.equal(spendingTotals(at, []).total, totals.total);
  assert.equal(categoryChoices([data], 'Political')[0].value, setToken('law/mod'));
});

test('a long mod category key stays selectable and filters rows without changing exact totals', () => {
  const category = 'mod_category_'.repeat(20);
  const data = reply({ spendCategories: [{ id: category, name: 'Long mod category', identified: true }], spending: [spend({ category })] });
  const [choice] = categoryChoices([data], 'Political');
  assert.equal(choice.label, 'Long mod category');
  assert.equal(choice.unavailable, undefined);
  assert.equal(choice.value, setToken(category));
  const at = spendingAt([data], 'Political', 12);
  assert.equal(spendingRows(at, [], '', 'newest').length, 1);
  assert.equal(spendingRows(at, [], choice.value, 'newest', 'include').length, 1);
  assert.equal(spendingRows(at, [], choice.value, 'newest').length, 0);
  assert.equal(spendingTotals(at, []).total, 123456789012345678901n);
});

test('unidentified cause presentation keeps its exact charge without guessing a category identity', () => {
  const data = validatePoliticsSpending(reply({ spendCategories: [{ id: 'recorded-unattributed-kind', name: 'Not identified', identified: false }],
    spending: [spend({ category: 'recorded-unattributed-kind' })] }));
  const [at] = spendingAt([data], 'Political', 12);
  assert.equal(at.identified, false);
  assert.equal(at.charge, 123456789012345678901n);
  assert.equal(spendingTotals([at], []).total, at.charge);
  assert.equal(categoryChoices([data], 'Political')[0].identified, false);
});

test('largest cost sorting remains exact beyond floating point precision', () => {
  const data = reply({ spending: [spend({ id: 'small', changes: [{ readHour: 12, chargeRaw: '9007199254740992', realizedDeltaRaw: '-1', occurrences: 1, covered: true }] }),
    spend({ id: 'big', changes: [{ readHour: 12, chargeRaw: '9007199254740993', realizedDeltaRaw: '-1', occurrences: 1, covered: true }] })] });
  const rows = spendingRows(spendingAt([data], 'Political', 12), [], '', 'cost');
  assert.equal(rows[0].item.kind === 'spend' ? rows[0].item.at.spend.id : '', 'big');
});

test('same-hour rows follow exact native sequence order in each date sort', () => {
  const data = reply({ spending: [spend({ id: 'later', sequence: '18446744073709551615' }), spend({ id: 'earlier', sequence: '9999999999999999999' })] });
  const at = spendingAt([data], 'Political', 12);
  const firstId = (order: 'newest' | 'oldest') => {
    const row = spendingRows(at, [], '', order)[0]; return row.item.kind === 'spend' ? row.item.at.spend.id : null;
  };
  assert.equal(firstId('newest'), 'later');
  assert.equal(firstId('oldest'), 'earlier');
});

test('5,000 countries are accepted without any player-status gate or selected-country cap in the kit', () => {
  const replies = Array.from({ length: 5000 }, (_, index) => validatePoliticsSpending(reply({ tag: `country ${index}`,
    spending: [spend({ countryTag: `country ${index}`, countryName: `Country ${index}` })] })));
  const at = spendingAt(replies, 'Political', 12);
  assert.equal(at.length, 5000);
  assert.equal(spendingRows(at, [], '', 'newest').length, 5000);
  assert.equal(spendingTotals(at, []).total, 123456789012345678901n * 5000n);
});

test('malformed identities refuse replies by shape; unrenderable mod art stays missing', () => {
  assert.throws(() => validatePoliticsSpending(reply({ spending: [spend({ itemId: 'unlisted' })] })), /malformed/);
  assert.throws(() => validatePoliticsSpending(reply({ spending: [spend({ changes: [] })] })), /malformed/);
  const missingIcon = { ...item() } as Partial<LedgerSpendItem>; delete missingIcon.icon;
  assert.throws(() => validatePoliticsSpending(reply({ spendItems: [missingIcon as LedgerSpendItem] })), /malformed/);
  const data = validatePoliticsSpending(reply({ spendItems: [{ ...item(), icon: 'file:///mod/portrait.png' }] }));
  assert.equal(data.spendItems[0].icon, null);
  assert.throws(() => validatePoliticsSpending(reply({ ideaTooltips: { ideas: [{}], artwork: [], modifiers: [], texts: [] } as never })), /malformed/);
  assert.throws(() => validatePoliticsSpending(reply({ spendItems: [{ ...item(), tooltipToken: 'missing-tooltip' }] })), /unlisted idea tooltip/);
  assert.throws(() => validatePoliticsSpending(reply({ commandHolds: [
    { readHour: 12, throughHour: 13, totalRaw: '1', holders: [], reason: null },
    { readHour: 13, throughHour: 14, totalRaw: '1', holders: [], reason: null },
  ] })), /malformed/);
  assert.throws(() => validatePoliticsSpending(reply({ commandHolds: [
    { readHour: 12, throughHour: 12, totalRaw: 'missing', holders: [], reason: null },
  ] })), /malformed/);
});
