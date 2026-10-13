import test from 'node:test';
import assert from 'node:assert/strict';
import type { LedgerSpend, LedgerSpendItem, PoliticsLedgerReply } from '../generated/dashboard';
import { categoryChoices, spendingAt, spendingCountryRequests, spendingGapNote, spendingGaps, spendingGroups, spendingItems, spendingSummary, spendingTableRows, spendLabel,
  validatePoliticsSpending } from './politicsSpending.ts';
import { MAX_NAME_HOURS } from './countryFacts.ts';
import { setToken } from './options.ts';
import { groupTree } from '../charts/sunburst/layout.ts';
import { sortRows } from '../charts/table/logic.ts';
import { formatPower } from './powerDecimals.ts';
import type { SpendingSelection } from './politicsSpending';
import { definition as politicalSpending } from '../widgets/political-power-spending/definition.ts';
import { definition as commandSpending } from '../widgets/command-power-spending/definition.ts';
import { definition as spendingBreakdown } from '../widgets/political-power-spending-breakdown/definition.ts';
import { NO_SPENDING_CATEGORIES, POLITICS_SPENDING_OPTIONS } from './politicsSpendingOptions.ts';
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
  assert.match(at.item.unknown!, /Some payment dates are unavailable/);
  assert.deepEqual(spendingSummary([at]), { spent: 0n, unknownCosts: 2, uncovered: false }, 'undated contributions are counted, not guessed');
});

test('group coverage clips to membership and an unread membership hour is named as unrecorded', () => {
  const data = reply({ spending: [], spendingCoverage: [{ readHour: 0, sinceHour: 0, throughHour: 30, political: false, command: false, reason: 'Native stream missing' }] });
  const selection: SpendingSelection = { matches: (_, hour) => hour >= 14 && hour <= 19 && hour !== 16,
    membership: new Map([[data.tag, [{ firstHour: 14, lastHour: 19 }]]]), unobserved: [{ firstHour: 16, lastHour: 16 }] };
  const gaps = spendingGaps([data], 'Political', { from: null, hour: 30 }, selection);
  assert.deepEqual(gaps.map(gap => [gap.from, gap.through]), [[14, 19], [16, 16]]);
  assert.match(spendingGapNote(gaps)!, /^Spending wasn’t recorded for 7 hours \(/);
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
  assert.equal(formatPower(at.charge!, 5), '1,234,567,890,123,456.78901');
  assert.equal(formatPower(at.charge!, 0), '1,234,567,890,123,457');
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
  assert.deepEqual(spendingSummary(earlier), { spent: 100000n, unknownCosts: 0, uncovered: false });
  const later = spendingAt([data], 'Command', 15);
  assert.equal(later.length, 1);
  assert.equal(later[0].charge, 400000n);
  assert.equal(later[0].occurrences, 4);
  assert.equal(later[0].covered, false);
  assert.deepEqual(spendingSummary(later), { spent: 400000n, unknownCosts: 0, uncovered: true });
});

test('unrecorded time and missing costs are named while everything recorded still draws', () => {
  const data = reply({ spending: [spend(), spend({ id: 'unpriced', chargeRaw: null, covered: true,
    changes: [{ readHour: 12, chargeRaw: null, realizedDeltaRaw: '-200000', occurrences: 1, covered: true }] })],
  spendingCoverage: [{ readHour: 12, sinceHour: 10, throughHour: 12, political: false, command: true, reason: 'Dropped observations' },
    { readHour: 13, sinceHour: 12, throughHour: 13, political: false, command: true, reason: null }] });
  const gaps = spendingGaps([data], 'Political', { from: null, hour: 13 });
  assert.deepEqual(gaps.map(gap => [gap.from, gap.through]), [[10, 13]], 'touching stretches join whatever their reasons');
  const at = spendingAt([data], 'Political', 13);
  assert.deepEqual(spendingSummary(at), { spent: 123456789012345678901n, unknownCosts: 1, uncovered: false });
  const tree = groupTree(spendingItems([data], 'Political', { from: null, hour: 13, end: 13 }), { id: 'all', label: 'Spent' });
  assert.equal(tree.valueUnknown, false, 'unrecorded time never hides the recorded spending');
  assert.equal(tree.finalUnknown, false);
  assert.equal(tree.exact, 123456789012345678901n);
  assert.equal(tree.children.some(child => child.key === 'coverage-gap'), false);
  const rows = spendingTableRows(at, '');
  assert.deepEqual(rows.map(row => row.amount), [123456789012345678901n, null], 'a missing cost is no row amount, never zero');
  assert.equal(rows[1].partial, true);
  const empty = reply({ spending: [], spendingCoverage: [] });
  assert.equal(spendingGaps([empty], 'Political', { from: null, hour: 12 }).length, 1);
});

test('the gap note gives the unrecorded time and dates once, in plain words', () => {
  const day = 43_800_000 + 24 * 365 * 1943;
  const stretch = (from: number, through: number) => ({ from, through, readHour: through, reason: 'any', tag: 'a' });
  assert.equal(spendingGapNote([]), null);
  const note = spendingGapNote([stretch(day + 11, day + 12), stretch(day + 11, day + 12), stretch(day + 30 * 24, day + 30 * 24 + 1)])!;
  assert.match(note, /^Spending wasn’t recorded for 4 hours \(.+, .+\); anything spent then isn’t counted\.$/);
  assert.match(spendingGapNote([stretch(day, day + 24 * 9)])!, /for 9 days/);
  assert.match(spendingGapNote([1, 2, 3, 4, 5].map(at => stretch(day + at * 48, day + at * 48)))!, /and 2 other dates\)/);
});

test('every mounted payment names its country and target at its own date, however many dates they span', () => {
  const rows = (count: number) => {
    const data = reply({ spendItems: [{ ...item(), targetTag: 'mod target' }],
      spending: Array.from({ length: count }, (_, index) => spend({ id: `charge:${index}`, hour: 10 + index * 24, readHour: 12 + index * 24,
        changes: [{ readHour: 12 + index * 24, chargeRaw: '100000', realizedDeltaRaw: '-100000', occurrences: 1, covered: true }] })) });
    return spendingTableRows(spendingAt([data], 'Political', count * 24), '');
  };
  const many = spendingCountryRequests(rows(MAX_NAME_HOURS + 36), true);
  assert.equal(many.wanted.length, 2 * (MAX_NAME_HOURS + 36));
  assert.equal(many.maxHours, MAX_NAME_HOURS + 36, 'the newest rows keep their names past the default hour limit');
  assert.deepEqual(many.wanted.slice(0, 2), [{ hour: 10, tag: 'mod country' }, { hour: 10, tag: 'mod target' }]);
  const one = spendingCountryRequests(rows(3), false);
  assert.deepEqual([one.wanted.length, one.maxHours], [3, MAX_NAME_HOURS], 'one country names only its targets');
  assert.deepEqual(spendingCountryRequests([], true), { wanted: [], maxHours: MAX_NAME_HOURS });
});

test('payments the recording could not name are named by their kind, never “Not identified”', () => {
  const raid = { ...item('raid'), name: null, action: 'Raids: Not identified', icon: null };
  const decision = { ...item('decision'), name: 'Mod decision', action: 'Mod decision: Not identified', targetTag: 'target', targetName: null };
  assert.equal(spendLabel(raid, 'Raids'), 'Raids');
  assert.equal(spendLabel(decision, 'Decisions'), 'Mod decision');
  assert.equal(spendLabel(item(), 'Mod laws'), 'A modded law');
  const data = reply({ spendItems: [raid], spending: [spend({ itemId: 'raid', category: 'raids' })],
    spendCategories: [{ id: 'raids', name: 'Raids', identified: true }] });
  const [row] = spendingTableRows(spendingAt([data], 'Political', 12), '');
  assert.equal(row.label, 'Raids');
  const [leaf] = spendingItems([data], 'Political', { from: null, hour: 12, end: 12 });
  assert.deepEqual(leaf.path.map(part => part.label), ['Raids'], 'no item slice repeats its category');
});

test('costs charged every day fold into one exact row per country, category and game month', () => {
  const first = 43_800_000 + 24 * 365 * 1944;
  const days = Array.from({ length: 62 }, (_, day) => spend({ id: `daily ${day}`, hour: first + day * 24, readHour: first + day * 24, category: 'dailyCosts',
    itemId: 'cost', changes: [{ readHour: first + day * 24, chargeRaw: '100001', realizedDeltaRaw: null, occurrences: 1, covered: true }] }));
  const data = reply({ spendItems: [item(), { ...item('cost'), name: 'Recorded cost modifier', action: 'Daily costs: Recorded cost modifier' }],
    spendCategories: [{ id: 'law/mod', name: 'Mod laws', identified: true }, { id: 'dailyCosts', name: 'Daily costs', identified: true }],
    spending: [...days, spend({ hour: first, readHour: first, changes: [{ readHour: first, chargeRaw: '100000', realizedDeltaRaw: '-100000', occurrences: 1, covered: true }] })] });
  const at = spendingAt([data], 'Political', first + 62 * 24);
  const rows = spendingTableRows(at, '');
  const folded = rows.filter(row => row.month !== null);
  assert.deepEqual(folded.map(row => [row.days, row.amount]), [[31, 3100031n], [28, 2800028n], [3, 300003n]]);
  assert.ok(folded.every(row => row.label === 'Daily costs' && row.parts.length === row.days));
  assert.equal(folded[0].hour, first + 30 * 24, 'a month shows its latest payment');
  assert.equal(rows.filter(row => row.month === null).length, 1);
  assert.equal(rows.reduce((sum, row) => sum + (row.amount ?? 0n), 0n), spendingSummary(at).spent, 'folding never changes the total');
});

test('Daily deficit remains its own recorded category alongside real costs', () => {
  const data = validatePoliticsSpending(reply({ spending: [spend({ id: 'deficit', currency: 'Command', category: 'daily-deficit' }),
    spend({ id: 'ability', currency: 'Command', category: 'daily-costs' })], spendCategories: [
    { id: 'daily-deficit', name: 'Daily deficit', identified: true }, { id: 'daily-costs', name: 'Daily costs', identified: true },
  ] }));
  const choices = categoryChoices([data], 'Command');
  assert.deepEqual(choices.map(choice => choice.label), ['Daily deficit', 'Daily costs'], 'no “None” choice');
  const rows = spendingTableRows(spendingAt([data], 'Command', 12), setToken('daily-costs'));
  assert.equal(rows.length, 1);
  assert.equal(rows[0].categoryName, 'Daily costs');
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

test('coverage uncertainty keeps known recorded charges and marks them partial', () => {
  const data = reply({ spending: [spend({ covered: false, changes: [{ readHour: 12, chargeRaw: '2500000', realizedDeltaRaw: '-2000000', occurrences: 1, covered: false }] })] });
  const [at] = spendingAt([data], 'Political', 12);
  assert.equal(at.charge, 2500000n);
  const [leaf] = spendingItems([data], 'Political', { from: null, hour: 12, end: 12 });
  assert.equal(leaf.exact, 2500000n);
  assert.equal(leaf.unknownValue, undefined, 'the recorded amount still draws');
  assert.deepEqual(leaf.unknown, { incomplete: 1 }, 'and is marked partial');
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
  assert.deepEqual(spendingSummary([at]), { spent: 250001n, unknownCosts: 1, uncovered: false });
  const [leaf] = spendingItems([data], 'Command', { from: null, hour: 13, end: 13 });
  assert.equal(leaf.exact, 250001n);
  assert.deepEqual(leaf.unknown, { incomplete: 1 });
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

test('category filters keep amounts exact and a legacy “none” filter shows every category', () => {
  const data = reply({ spending: [spend(), spend({ id: 'second', category: 'advisor' })],
    spendCategories: [{ id: 'law/mod', name: 'Mod laws', identified: true }, { id: 'advisor', name: 'Advisors', identified: true }] });
  const at = spendingAt([data], 'Political', 12);
  assert.equal(spendingTableRows(at, '').length, 2);
  assert.equal(spendingTableRows(at, setToken('advisor')).length, 1);
  assert.equal(spendingTableRows(at, NO_SPENDING_CATEGORIES).length, 2);
  assert.equal(spendingSummary(at).spent, 123456789012345678901n * 2n, 'the summary counts every category');
  assert.equal(categoryChoices([data], 'Political')[0].value, setToken('law/mod'));
});

test('authored empty and key-equal category titles remain valid across choices and spending views', () => {
  for (const name of ['', 'law/mod']) {
    const data = validatePoliticsSpending(reply({ spendCategories: [{ id: 'law/mod', name, identified: true }] }));
    const [choice] = categoryChoices([data], 'Political');
    assert.equal(choice.label, name);
    assert.equal(choice.value, setToken('law/mod'));
    const at = spendingAt([data], 'Political', 12);
    assert.equal(at[0].category, name);
    assert.equal(at[0].charge, 123456789012345678901n);
    const [leaf] = spendingItems([data], 'Political', { from: null, hour: 12, end: 12 });
    assert.equal(leaf.path[0].label, name);
    assert.equal(spendingSummary(at).spent, 123456789012345678901n);
  }
  assert.throws(() => validatePoliticsSpending(reply({ spendCategories: [{ id: 'law/mod', name: null as never, identified: true }] })), /malformed/);
});

test('a long mod category key stays selectable and filters rows without changing exact totals', () => {
  const category = 'mod_category_'.repeat(20);
  const data = reply({ spendCategories: [{ id: category, name: 'Long mod category', identified: true }], spending: [spend({ category })] });
  const [choice] = categoryChoices([data], 'Political');
  assert.equal(choice.label, 'Long mod category');
  assert.equal(choice.unavailable, undefined);
  assert.equal(choice.value, setToken(category));
  const at = spendingAt([data], 'Political', 12);
  assert.equal(spendingTableRows(at, '').length, 1);
  assert.equal(spendingTableRows(at, choice.value).length, 1);
  assert.equal(spendingTableRows(at, setToken('another kind')).length, 0);
  assert.equal(spendingSummary(at).spent, 123456789012345678901n);
});

test('unidentified cause presentation keeps its exact charge without guessing a category identity', () => {
  const data = validatePoliticsSpending(reply({ spendCategories: [{ id: 'recorded-unattributed-kind', name: 'Not identified', identified: false }],
    spending: [spend({ category: 'recorded-unattributed-kind' })] }));
  const [at] = spendingAt([data], 'Political', 12);
  assert.equal(at.identified, false);
  assert.equal(at.charge, 123456789012345678901n);
  assert.equal(spendingSummary([at]).spent, at.charge);
  assert.deepEqual(categoryChoices([data], 'Political').map(choice => [choice.label, choice.identified]), [['Other spending', false]]);
  assert.equal(spendingTableRows([at], '')[0].categoryName, 'Other spending');
  assert.equal(spendingItems([data], 'Political', { from: null, hour: 12, end: 12 })[0].path[0].label, 'Other spending');
});

test('largest cost sorting remains exact beyond floating point precision', () => {
  const data = reply({ spending: [spend({ id: 'small', changes: [{ readHour: 12, chargeRaw: '9007199254740992', realizedDeltaRaw: '-1', occurrences: 1, covered: true }] }),
    spend({ id: 'big', changes: [{ readHour: 12, chargeRaw: '9007199254740993', realizedDeltaRaw: '-1', occurrences: 1, covered: true }] })] });
  const rows = spendingTableRows(spendingAt([data], 'Political', 12), '');
  const key = (row: (typeof rows)[number]) => row.key;
  assert.equal(sortRows(rows, row => row.amount, 'descending', key)[0].parts[0].spend.id, 'big');
});

test('same-hour rows follow exact native sequence order in each date sort', () => {
  const data = reply({ spending: [spend({ id: 'later', sequence: '18446744073709551615' }), spend({ id: 'earlier', sequence: '9999999999999999999' })] });
  const rows = spendingTableRows(spendingAt([data], 'Political', 12), '');
  const key = (row: (typeof rows)[number]) => row.key;
  assert.equal(sortRows(rows, row => row.order, 'descending', key)[0].parts[0].spend.id, 'later');
  assert.equal(sortRows(rows, row => row.order, 'ascending', key)[0].parts[0].spend.id, 'earlier');
});

test('5,000 countries are accepted without any player-status gate or selected-country cap in the kit', () => {
  const replies = Array.from({ length: 5000 }, (_, index) => validatePoliticsSpending(reply({ tag: `country ${index}`,
    spending: [spend({ countryTag: `country ${index}`, countryName: `Country ${index}` })] })));
  const at = spendingAt(replies, 'Political', 12);
  assert.equal(at.length, 5000);
  assert.equal(spendingTableRows(at, '').length, 5000);
  assert.equal(spendingSummary(at).spent, 123456789012345678901n * 5000n);
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
