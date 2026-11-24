import assert from 'node:assert/strict';
import test from 'node:test';
import { tooltipText } from '../../charts/tooltip.ts';
import { createBoundedReadView, displayedHour, type FamilyTheme } from '../../families/contract.ts';
import { buildBreakdownTooltip, resolveDrillLevel, type BreakdownPart } from '../../families/breakdown/model.ts';
import { constructionSpendingProjection } from './constructionSpendingProjection.ts';
import { footnote, formatSpent, hasConstruction, presentKey, spendingItems, validateSpendingReply } from './spendingData.ts';
import { OPTIONS } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { icDecimals } from '../../dashboard/icDecimals.ts';
import type { ConstructionSpendingReply, SpendingLeaf } from '../../generated/dashboard';

const THEME = {
  surface: '#11141a', text: '#e8edf5', muted: '#8c96a6', line: '#2a303c', lineSoft: '#222832',
  accent: '#4f8ff7', blue: '#4f8ff7', red: '#e05a5a', amber: '#d6a244', green: '#45a86b', grey: '#6e7785',
  hatch: '#333b48', font: 'sans-serif', fontSize: { small: 11, body: 12 }, lineWidth: 1, hatchSpacing: 6,
} as FamilyTheme;
const project = (data: ConstructionSpendingReply, hour: number, extra: { from?: number | null; digits?: number; levels?: readonly boolean[] } = {}) =>
  constructionSpendingProjection.project(
    createBoundedReadView({ hour: displayedHour(hour), panels: new Map([['spending', data]]) }),
    { from: extra.from ?? null, firstHour: 100, digits: extra.digits ?? 0, levels: extra.levels ?? [true, true, true] },
  );

const leaf = (patch: Partial<SpendingLeaf> = {}): SpendingLeaf => ({
  repair: false, kind: 'building', building: 777, state: 5, toState: null,
  spent: [], unknown: [], projects: [{ project: 'p', firstRead: 100, until: null }], ...patch,
});
const reply = (leaves: SpendingLeaf[] = []): ConstructionSpendingReply => ({
  branchId: '1', tag: 'synthetic-country', costScale: 100000, recorded: true, repairIcon: null, beforeFirstRead: 0, warnings: [], unattributed: [], missing: [], leaves,
  types: [{ id: 777, token: 'synthetic_mod_building', name: 'Synthetic building', icon: null }], states: [{ id: 5, name: 'Synthetic state' }],
});
const WHOLE = { from: null, hour: 148, end: 148 };

test('spending names both railway endpoints from the reply, and a state the content does not name keeps its id and amount', () => {
  const data = reply([leaf({ kind: 'railway', toState: 6, spent: [{ knownAt: 124, totalRaw: '99' }] })]);
  data.states = [{ id: 5, name: 'Stored owner name' }, { id: 6, name: 'Other state' }];
  const known = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.equal(known.path[2].label, 'Stored owner name to Other state');
  assert.equal(known.exact, 99n);
  data.states = [{ id: 5, name: null }];
  const missing = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.equal(missing.path[2].label, 'State 5 to State 6');
  assert.equal(missing.path[2].id, known.path[2].id);
  assert.equal(missing.exact, known.exact);
  const note = 'State 5: no name in the recorded content State 6: not named by the reply';
  assert.equal(missing.note, note);
  for (const levels of [[true, true, true], [false, true, false]]) {
    const model = project(data, 124, { digits: 5, levels });
    let node: BreakdownPart = model.parts[0]!;
    while (node.children?.length) node = node.children[0]!;
    assert.equal(node.text, '0.00099');
    assert.equal(node.notRecorded, undefined);
    const tip = buildBreakdownTooltip(model, resolveDrillLevel(model, []), node.key, THEME)!;
    assert.equal(tip.rows[0].value, '0.00099 IC');
    assert.equal(tip.note, note);
    assert.ok(tooltipText(tip).endsWith(`; ${note}`));
  }
});

test('recorded repairs and signed corrections remain exact selectable amounts', () => {
  const data = reply([
    leaf({ spent: [{ knownAt: 124, totalRaw: '9007199254741499' }, { knownAt: 148, totalRaw: '9007199254741498' }] }),
    leaf({ repair: true, spent: [{ knownAt: 124, totalRaw: '250001' }] }),
  ]);
  assert.equal(validateSpendingReply(data), data);
  const items = spendingItems(data, { from: null, hour: 124, end: 148 });
  assert.deepEqual(items[1].path.map(part => part.label), ['Repairs', 'Synthetic building', 'Synthetic state']);
  assert.equal(items[1].exact, 250001n);
  const correction = spendingItems(data, { from: 125, hour: 148, end: 148 })[0];
  assert.equal(correction.exact, -1n);
  assert.equal(correction.value, -1);
  assert.equal(correction.present, true);
  assert.equal(formatSpent(correction.value!, correction.exact!, 100000, 5), '−0.00001');
});

test('a total the recording does not hold is not measured, never zero, and its earlier amounts stay', () => {
  const data = reply([leaf({ spent: [{ knownAt: 100, totalRaw: '300' }, { knownAt: 148, totalRaw: null }] })]);
  const before = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.deepEqual([before.exact, before.value, before.unknownValue, before.final, before.finalUnknown], [300n, 300, false, null, true]);
  const after = spendingItems(data, { from: null, hour: 148, end: 148 })[0];
  assert.deepEqual([after.exact, after.value, after.unknownValue], [null, null, true]);
  assert.deepEqual(after.unknown, { notRecorded: 1 });
  assert.equal(project(data, 148).total.atLeast, true);
  assert.equal(project(data, 148).parts[0]!.notRecorded, true);
  assert.equal(footnote(data, [after], WHOLE), 'Partial amounts leave out output that wasn’t recorded exactly.');
});

test('output not recorded for a stretch leaves the counted amount partial until it is known again', () => {
  const data = reply([leaf({ spent: [{ knownAt: 100, totalRaw: '300' }, { knownAt: 148, totalRaw: '700' }],
    unknown: [{ knownAt: 124, reason: 'notRecorded', project: 'p', until: 148 }] })]);
  const missing = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.ok(!missing.unknownValue);
  assert.equal(missing.exact, 300n);
  assert.deepEqual(missing.unknown, { notRecorded: 1 });
  assert.equal(missing.finalUnknown, false);
  const known = spendingItems(data, WHOLE)[0];
  assert.ok(!known.unknownValue);
  assert.deepEqual(known.unknown, {});
  assert.equal(known.exact, 700n);
});

test('linked output keeps what was counted and makes the whole partial, including a later range', () => {
  const data = reply([leaf({ spent: [{ knownAt: 100, totalRaw: '90' }],
    unknown: [{ knownAt: 124, reason: 'linkedOutput', project: 'p', until: null }] })]);
  const whole = project(data, 148, { digits: 5 });
  assert.equal(whole.total.text, '0.0009');
  assert.equal(whole.total.partial, true);
  const later = project(data, 148, { from: 130, digits: 5 });
  assert.equal(later.total.partial, true);
});

test('ownerless output makes only the whole partial, in ranges that include it', () => {
  const data = reply([leaf({ spent: [{ knownAt: 148, totalRaw: '70' }] })]);
  data.unattributed = [{ knownAt: 124, reason: 'ownerUnknown', project: '99', until: 125 }];
  const early = spendingItems(data, { from: null, hour: 100, end: 148 });
  assert.equal(early.at(-1)?.present, false);
  assert.equal(early.at(-1)?.finalUnknown, true);
  const whole = spendingItems(data, WHOLE);
  assert.deepEqual(whole.at(-1)?.path, []);
  assert.deepEqual(whole.at(-1)?.unknown, { ownerUnknown: 1 });
  const model = project(data, 148, { digits: 5 });
  assert.equal(model.total.text, '0.0007');
  assert.equal(model.total.partial, true);
  assert.deepEqual(model.parts.map(child => child.name), ['New buildings']);
  assert.equal(footnote(data, whole, WHOLE), 'Output of 1 project not traced to a country isn’t counted.');
  const window = { from: 125, hour: 148, end: 148 };
  const later = spendingItems(data, window);
  assert.equal(later.at(-1)?.present, false);
  assert.equal(later.at(-1)?.finalUnknown, false);
  assert.equal(footnote(data, later, window), undefined);
});

test('known amounts stay drawn and listed when output elsewhere is not counted', () => {
  const data = reply([
    leaf({ spent: [{ knownAt: 124, totalRaw: '23049375000' }] }),
    leaf({ state: 6, spent: [{ knownAt: 100, totalRaw: '100000' }], unknown: [{ knownAt: 124, reason: 'inexactOutput', project: 'q', until: null }],
      projects: [{ project: 'q', firstRead: 100, until: null }] }),
    leaf({ repair: true, spent: [{ knownAt: 124, totalRaw: '916375000' }] }),
  ]);
  data.unattributed = [{ knownAt: 124, reason: 'ownerUnknown', project: '99', until: null }];
  data.missing = [{ firstHour: 130, lastHour: 140, reason: 'The recording holds no hours here' }];
  data.beforeFirstRead = 15;
  const items = spendingItems(data, WHOLE);
  assert.ok(hasConstruction(items));
  const model = project(data, 148);
  assert.equal(model.total.partial, true);
  assert.equal(model.total.value, Number(23049375000n + 100000n + 916375000n));
  assert.deepEqual(model.parts.map(child => child.name), ['New buildings', 'Repairs']);
  assert.ok(model.parts.every(child => !child.notRecorded));
  const tip = buildBreakdownTooltip(model, resolveDrillLevel(model, []), model.parts[0]!.key, THEME)!;
  assert.equal(tip.rows[0].value, '230,495 IC');
  assert.equal(model.parts[0]!.partial, true);
  assert.equal(footnote(data, items, WHOLE), 'Earlier spending on 15 projects isn’t counted. Partial amounts leave out output that wasn’t recorded exactly. '
    + 'Output of 1 project not traced to a country isn’t counted. Output in hours that weren’t recorded isn’t counted.');
  const owned = { ...data, beforeFirstRead: 0, unattributed: [] };
  assert.equal(footnote(owned, spendingItems(owned, { from: null, hour: 124, end: 148 }), { from: null, hour: 124, end: 148 }),
    'Partial amounts leave out output that wasn’t recorded exactly.', 'hours not held after the hour shown are not named yet');
});

test('a country with only output no country can be given shows no construction', () => {
  const data = reply([]);
  data.unattributed = [{ knownAt: 124, reason: 'ownerUnknown', project: '99', until: null }];
  assert.equal(hasConstruction(spendingItems(data, WHOLE)), false);
});

test('spent IC is written in whole numbers unless the settings ask for decimals', () => {
  const written = (stored: Record<string, unknown>) => formatSpent(Number(23049375000n), 23049375000n, 100000, icDecimals(resolveOptions(OPTIONS, stored as never), 5));
  assert.equal(written({}), '230,494');
  assert.equal(written({ icDecimals: 2 }), '230,493.75');
  assert.equal(written({ icDecimals: 5 }), '230,493.75');
});

test('a leaf does not appear before its first line is read, and the render key moves only when one appears', () => {
  const data = reply([leaf({ projects: [{ project: 'p', firstRead: 148, until: null }], spent: [{ knownAt: 148, totalRaw: '100' }] }),
    leaf({ state: 6, spent: [{ knownAt: 120, totalRaw: '5' }, { knownAt: 130, totalRaw: '9' }] })]);
  const at = (hour: number) => spendingItems(data, { from: null, hour, end: 148 });
  const item = at(124)[0];
  assert.equal(item.present, false);
  assert.equal(item.final, 100);
  assert.equal(presentKey(at(124)), presentKey(at(140)));
  assert.notEqual(presentKey(at(140)), presentKey(at(148)));
});

test('hiding the kind level joins totals without duplicating a project', () => {
  const data = reply([
    leaf({ spent: [{ knownAt: 124, totalRaw: '90' }] }),
    leaf({ repair: true, spent: [{ knownAt: 124, totalRaw: '10' }] }),
  ]);
  const model = project(data, 124, { digits: 5, levels: [false, true, false] });
  assert.equal(model.parts[0]!.text, '0.001');
  assert.equal(model.parts[0]!.value, 100);
});

test('the output contract refuses malformed steps, dates and reasons the recording does not give', () => {
  const data = reply([leaf({ spent: [{ knownAt: 124, totalRaw: '10' }] })]);
  assert.equal(validateSpendingReply(data), data);
  assert.throws(() => validateSpendingReply({ ...data, unattributed: undefined }), /malformed/);
  assert.throws(() => validateSpendingReply({ ...data, missing: undefined }), /malformed/);
  const wrong = structuredClone(data);
  wrong.leaves[0].unknown = [{ knownAt: 124, reason: 'notRecorded', project: 'p', until: 100 }];
  assert.throws(() => validateSpendingReply(wrong), /malformed/);
  wrong.leaves[0].unknown = [{ knownAt: 124, reason: 'readFailed' as never, project: 'p', until: null }];
  assert.throws(() => validateSpendingReply(wrong), /malformed/);
  wrong.leaves[0].unknown = [];
  wrong.leaves[0].spent[0].totalRaw = 'not a number';
  assert.throws(() => validateSpendingReply(wrong), /malformed/);
  assert.equal(footnote({ ...data, beforeFirstRead: 2 }, [], WHOLE), 'Earlier spending on 2 projects isn’t counted.');
});

test('state rows retain name reasons beside exact recorded output', () => {
  const data = reply([5, 6, 7].map((state, index) => leaf({ state,
    spent: [{ knownAt: 124, totalRaw: String((index + 1) * 100) }],
    projects: [{ project: `p${state}`, firstRead: 100, until: null }],
  })));
  data.states = [{ id: 5, name: null }, { id: 6, name: null }, { id: 7, name: null }];
  const model = project(data, 124, { digits: 5 });
  const states = model.parts[0]!.children![0]!.children!;
  assert.equal(model.total.text, '0.006');
  assert.equal(model.total.known, true);
  assert.equal(states.length, 3);
  const state6 = states.find(node => node.name === 'State 6')!;
  const tip = buildBreakdownTooltip(model, resolveDrillLevel(model, []), state6.key, THEME)!;
  assert.equal(tip.rows[0].value, '0.002 IC');
  assert.equal(tip.note, 'State 6: no name in the recorded content');
  assert.ok(tooltipText(tip).endsWith('; State 6: no name in the recorded content'));
});
