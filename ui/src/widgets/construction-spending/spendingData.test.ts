import assert from 'node:assert/strict';
import test from 'node:test';
import type { ConstructionSpendingReply, SpendingLeaf } from '../../generated/dashboard';
import { arcTargets, foldTree, groupTree, listed, sliceTooltip, unknownTotal } from '../../charts/sunburst/layout.ts';
import { tooltipText } from '../../charts/tooltip.ts';
import { footnote, formatSpent, hasConstruction, spendingItems as datedSpendingItems, validateSpendingReply } from './spendingData.ts';
import { OPTIONS } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { icDecimals } from '../../dashboard/icDecimals.ts';
import type { SpendingWindow } from './spendingData';
import type { StateNameAt } from '../../dashboard/construction';

const leaf = (patch: Partial<SpendingLeaf> = {}): SpendingLeaf => ({
  repair: false, kind: 'building', building: 0, state: 5, toState: null,
  spent: [], unknown: [], projects: [{ project: 'p', firstRead: 100, knownAt: null, until: null, unread: false, resumes: null }], ...patch,
});
const reply = (leaves: SpendingLeaf[] = []): ConstructionSpendingReply => ({
  branchId: '1', tag: 'synthetic-country', costScale: 100000, reads: [100, 124, 148], railwayReads: [100, 124, 148],
  repairIcon: null, beforeFirstRead: 0, warnings: [], unattributed: [], leaves,
  types: [{ id: 777, token: 'synthetic_mod_building', name: 'Synthetic building', icon: null }], states: [{ id: 5, name: 'Synthetic state' }],
});
const fixtureNames: StateNameAt = (_hour, state) => ({ state, name: state === 5 ? 'Synthetic state' : null, missing: null });
const spendingItems = (data: ConstructionSpendingReply, window: SpendingWindow) => datedSpendingItems(data, window, fixtureNames);

test('spending names both railway endpoints at the shown hour while totals and identities survive renames', () => {
  const data = reply([leaf({ kind: 'railway', toState: 6, spent: [{ knownAt: 124, totalRaw: '99' }] })]);
  data.states = [{ id: 5, name: 'Future authored label' }, { id: 6, name: 'Future other label' }];
  const requested: [number, number][] = [];
  const names: StateNameAt = (hour, state) => {
    requested.push([hour, state]);
    return { state, name: hour === 124 ? state === 5 ? 'Stored owner name' : 'Other state' : null,
      missing: hour === 130 ? 'States read failed' : hour === 148 ? 'Recorded language does not match' : null };
  };
  const known = datedSpendingItems(data, { from: null, hour: 124, end: 148 }, names)[0];
  assert.equal(known.path[2].label, 'Stored owner name to Other state');
  assert.equal(known.exact, 99n);
  for (const [hour, reason] of [[130, 'States read failed'], [148, 'Recorded language does not match']] as const) {
    const missing = datedSpendingItems(data, { from: null, hour, end: 148 }, names)[0];
    assert.equal(missing.path[2].label, 'State 5 to State 6');
    assert.equal(missing.path[2].id, known.path[2].id);
    assert.equal(missing.exact, known.exact);
    const note = `State 5: ${reason} State 6: ${reason}`;
    assert.equal(missing.note, note);
    assert.doesNotMatch(missing.note, /State name unavailable/);
    // Keep the real shared tree/tooltip path: a missing name is not missing IC.
    const text = { format: (value: number, exact: bigint | null) => formatSpent(value, exact, data.costScale, 5), unit: 'IC', valueLabel: 'Spent' };
    for (const levels of [[true, true, true], [false, true, false], [false, false, false]]) {
      const root = groupTree([missing], { id: 'all', label: 'All construction' }, levels);
      let node = root;
      while (node.children.length) node = node.children[0];
      assert.equal(node.exact, 99n);
      assert.equal(node.valueUnknown, false);
      for (const tip of sliceTooltip(node, root, text)) {
        assert.equal(tip.rows[0].value, '0.00099 IC');
        assert.equal(tip.note, note);
        assert.ok(tooltipText(tip).endsWith(`; ${note}`));
        assert.doesNotMatch(tooltipText(tip), /State name unavailable/);
      }
    }
  }
  assert.deepEqual(requested, [[124, 5], [124, 6], [130, 5], [130, 6], [148, 5], [148, 6]]);
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

test('a failed observation leaves the counted amount partial until an exact cumulative read arrives', () => {
  const data = reply([leaf({ spent: [{ knownAt: 100, totalRaw: '300' }, { knownAt: 148, totalRaw: '700' }],
    unknown: [{ knownAt: 124, reason: 'readFailed', project: 'p', until: 148 }] })]);
  const missing = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.ok(!missing.unknownValue);
  assert.equal(missing.exact, 300n);
  assert.deepEqual(missing.unknown, { readFailed: 1 });
  assert.equal(missing.finalUnknown, false);
  const known = spendingItems(data, { from: null, hour: 148, end: 148 })[0];
  assert.ok(!known.unknownValue);
  assert.deepEqual(known.unknown, {});
  assert.equal(known.exact, 700n);
});

test('linked output keeps what was counted and makes the whole partial, including a later range', () => {
  const data = reply([leaf({ spent: [{ knownAt: 100, totalRaw: '90' }],
    unknown: [{ knownAt: 124, reason: 'linkedOutput', project: 'p', until: null }] })]);
  const whole = groupTree(spendingItems(data, { from: null, hour: 148, end: 148 }), { id: 'all', label: 'All' });
  assert.equal(whole.valueUnknown, false);
  assert.equal(whole.exact, 90n);
  assert.deepEqual(whole.unknown, { linkedOutput: 1 });
  assert.equal(whole.finalUnknown, true);
  const later = groupTree(spendingItems(data, { from: 130, hour: 148, end: 148 }), { id: 'all', label: 'All' });
  assert.equal(later.exact, 0n);
  assert.deepEqual(later.unknown, { linkedOutput: 1 });
});

test('a prepared prefix keeps its totals through its last hour, partial with the output still being prepared', () => {
  // As the server answers a request for hour 148 from a prefix through 130.
  const data = { ...reply([leaf({ spent: [{ knownAt: 130, totalRaw: '70' }],
    unknown: [{ knownAt: 131, reason: 'preparing', project: '', until: null }] })]),
  coverage: { schema: 24, firstHour: 100, lastHour: 130, complete: false }, pending: [{ firstHour: 131, lastHour: 148 }] };
  validateSpendingReply(data);
  const items = spendingItems(data, { from: null, hour: 148, end: 148 });
  assert.equal(items[0].exact, 70n);
  assert.deepEqual(items[0].unknown, { preparing: 1 });
  assert.equal(items[0].finalUnknown, true);
  assert.equal(spendingItems(data, { from: null, hour: 130, end: 130 })[0].finalUnknown, false, 'the prefix itself is exact');
  assert.equal(footnote(data, items), 'Partial amounts leave out output still being prepared.');
  const dated = 'Spent through 1 Jan; the rest is still being prepared.';
  assert.equal(footnote(data, items, dated), dated, 'the dated note replaces the partial amounts’ preparing note');
  assert.equal(footnote({ ...data, beforeFirstRead: 2 }, [], dated), `Earlier spending on 2 projects isn’t counted. ${dated}`);
});

test('ownerless output makes only the whole partial, in ranges that include it', () => {
  const data = reply([leaf({ spent: [{ knownAt: 148, totalRaw: '70' }] })]);
  data.unattributed = [{ knownAt: 124, reason: 'ownerUnknown', project: '99', until: 125 }];
  const early = spendingItems(data, { from: null, hour: 100, end: 148 });
  assert.equal(early.at(-1)?.present, false);
  assert.equal(early.at(-1)?.finalUnknown, true);
  const whole = spendingItems(data, { from: null, hour: 148, end: 148 });
  assert.deepEqual(whole.at(-1)?.path, []);
  assert.deepEqual(whole.at(-1)?.unknown, { ownerUnknown: 1 });
  const tree = groupTree(whole, { id: 'all', label: 'All' });
  assert.equal(tree.valueUnknown, false);
  assert.equal(tree.exact, 70n);
  assert.deepEqual(tree.unknown, { ownerUnknown: 1 });
  assert.deepEqual(tree.children.map(child => child.label), ['New buildings']);
  assert.equal(footnote(data, whole), 'Output of 1 project not traced to a country isn’t counted.');
  const later = spendingItems(data, { from: 125, hour: 148, end: 148 });
  assert.equal(later.at(-1)?.present, false);
  assert.equal(later.at(-1)?.finalUnknown, false);
  assert.equal(footnote(data, later), undefined);
});

test('known amounts stay drawn and listed when output elsewhere is not counted', () => {
  // As recorded for a country: new buildings and repairs, a project whose output
  // was not recorded exactly, and branch-wide output with no country.
  const data = reply([
    leaf({ spent: [{ knownAt: 124, totalRaw: '23049375000' }] }),
    leaf({ state: 6, spent: [{ knownAt: 100, totalRaw: '100000' }], unknown: [{ knownAt: 124, reason: 'readFailed', project: 'q', until: null }],
      projects: [{ project: 'q', firstRead: 100, knownAt: null, until: null, unread: false, resumes: null }] }),
    leaf({ repair: true, spent: [{ knownAt: 124, totalRaw: '916375000' }] }),
  ]);
  data.unattributed = [{ knownAt: 124, reason: 'ownerUnknown', project: '99', until: null }, { knownAt: 130, reason: 'notRecorded', project: '', until: null }];
  data.beforeFirstRead = 15;
  const items = spendingItems(data, { from: null, hour: 148, end: 148 });
  assert.ok(hasConstruction(items));
  const root = groupTree(items, { id: 'all', label: 'All construction' });
  assert.equal(root.valueUnknown, false);
  assert.equal(root.exact, 23049375000n + 100000n + 916375000n);
  assert.equal(unknownTotal(root), 3);
  // Only recorded kinds are listed: nothing named for output no country can be given.
  assert.deepEqual(root.children.filter(listed).map(child => child.label), ['New buildings', 'Repairs']);
  assert.ok(root.children.every(child => !child.valueUnknown));
  const targets = arcTargets(root, root, 'share');
  for (const child of root.children) assert.equal(targets.get(child.id)?.shown, true, child.label);
  const text = { format: (value: number, exact: bigint | null) => formatSpent(value, exact, data.costScale, 0), unit: 'IC', valueLabel: 'Spent' };
  assert.equal(sliceTooltip(root.children[0], root, text)[0].rows[0].value, '230,495 IC · partial');
  assert.equal(footnote(data, items), 'Earlier spending on 15 projects isn’t counted. Partial amounts leave out output that wasn’t recorded exactly. '
    + 'Output of 1 project not traced to a country isn’t counted. Output in hours that weren’t recorded isn’t counted.');
});

test('a country with only output no country can be given shows no construction', () => {
  const data = reply([]);
  data.unattributed = [{ knownAt: 124, reason: 'ownerUnknown', project: '99', until: null }];
  assert.equal(hasConstruction(spendingItems(data, { from: null, hour: 148, end: 148 })), false);
});

test('spent IC is written in whole numbers unless the settings ask for decimals', () => {
  const written = (stored: Record<string, unknown>) => formatSpent(Number(23049375000n), 23049375000n, 100000, icDecimals(resolveOptions(OPTIONS, stored as never), 5));
  assert.equal(written({}), '230,494');
  assert.equal(written({ icDecimals: 2 }), '230,493.75');
  assert.equal(written({ icDecimals: 5 }), '230,493.75');
});

test('future content leaves do not appear before their supplying read', () => {
  const data = reply([leaf({ projects: [{ project: 'p', firstRead: 148, knownAt: null, until: null, unread: false, resumes: null }],
    spent: [{ knownAt: 148, totalRaw: '100' }] })]);
  const item = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.equal(item.present, false);
  assert.equal(item.final, 100);
});

test('hiding the kind level joins totals without duplicating a project', () => {
  const data = reply([
    leaf({ spent: [{ knownAt: 124, totalRaw: '90' }] }),
    leaf({ repair: true, spent: [{ knownAt: 124, totalRaw: '10' }] }),
  ]);
  const tree = groupTree(spendingItems(data, { from: null, hour: 124, end: 148 }), { id: 'all', label: 'All' }, [false, true, false]);
  assert.equal(tree.children[0].exact, 100n);
  assert.equal(tree.children[0].count, 1);
});

test('the output contract refuses malformed steps and dates', () => {
  const data = reply([leaf({ spent: [{ knownAt: 124, totalRaw: '10' }] })]);
  assert.equal(validateSpendingReply(data), data);
  assert.throws(() => validateSpendingReply({ ...data, unattributed: undefined }), /malformed/);
  const wrong = structuredClone(data);
  wrong.leaves[0].unknown = [{ knownAt: 124, reason: 'readFailed', project: 'p', until: 100 }];
  assert.throws(() => validateSpendingReply(wrong), /malformed/);
  wrong.leaves[0].unknown = [];
  wrong.leaves[0].spent[0].totalRaw = 'not a number';
  assert.throws(() => validateSpendingReply(wrong), /malformed/);
  assert.equal(footnote({ ...data, beforeFirstRead: 2 }, []), 'Earlier spending on 2 projects isn’t counted.');
});


test('folded state rows retain name failure reasons beside exact recorded output', () => {
  const data = reply([5, 6, 7].map((state, index) => leaf({ state,
    spent: [{ knownAt: 124, totalRaw: String((index + 1) * 100) }],
    projects: [{ project: `p${state}`, firstRead: 100, knownAt: null, until: null, unread: false, resumes: null }],
  })));
  const names: StateNameAt = (_hour, state) => ({ state, name: null, missing: 'Stored state name language differs from recorded language' });
  const items = datedSpendingItems(data, { from: null, hour: 124, end: 148 }, names);
  const root = foldTree(groupTree(items, { id: 'all', label: 'All construction' }), 2);
  const states = root.children[0].children[0].children;
  const other = states.find(node => node.other)!;
  assert.equal(root.exact, 600n);
  assert.equal(root.valueUnknown, false);
  assert.equal(other.exact, 300n);
  const text = { format: (value: number, exact: bigint | null) => formatSpent(value, exact, data.costScale, 5), unit: 'IC', valueLabel: 'Spent' };
  for (const tip of sliceTooltip(other, root, text)) {
    assert.equal(tip.rows[0].value, '0.003 IC');
    const note = 'State 6: Stored state name language differs from recorded language State 5: Stored state name language differs from recorded language';
    assert.equal(tip.note, note);
    assert.ok(tooltipText(tip).endsWith(`; ${note}`));
    assert.doesNotMatch(tooltipText(tip), /State name unavailable/);
  }
});
