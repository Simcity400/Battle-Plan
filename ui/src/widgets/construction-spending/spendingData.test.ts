import assert from 'node:assert/strict';
import test from 'node:test';
import type { ConstructionSpendingReply, SpendingLeaf } from '../../generated/dashboard';
import { foldTree, groupTree, sliceTooltip } from '../../charts/sunburst/layout.ts';
import { tooltipText } from '../../charts/tooltip.ts';
import { footnote, formatSpent, spendingItems as datedSpendingItems, validateSpendingReply } from './spendingData.ts';
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
  for (const [hour, reason] of [[130, 'States read failed'], [148, 'language does not match']] as const) {
    const missing = datedSpendingItems(data, { from: null, hour, end: 148 }, names)[0];
    assert.equal(missing.path[2].label, 'State 5 to State 6');
    assert.equal(missing.path[2].id, known.path[2].id);
    assert.equal(missing.exact, known.exact);
    assert.equal(missing.note, 'State name unavailable.');
    assert.doesNotMatch(missing.note, new RegExp(reason));
    // Keep the real shared tree/tooltip path: a missing name is not missing IC.
    const text = { format: (value: number, exact: bigint | null) => formatSpent(value, exact, data.costScale), unit: 'IC', valueLabel: 'Spent' };
    for (const levels of [[true, true, true], [false, true, false], [false, false, false]]) {
      const root = groupTree([missing], { id: 'all', label: 'All construction' }, levels);
      let node = root;
      while (node.children.length) node = node.children[0];
      assert.equal(node.exact, 99n);
      assert.equal(node.valueUnknown, false);
      for (const tip of sliceTooltip(node, root, text)) {
        assert.equal(tip.rows[0].value, '0.00099 IC');
        assert.match(tooltipText(tip), /State name unavailable/);
        assert.doesNotMatch(tooltipText(tip), new RegExp(reason));
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
  assert.equal(formatSpent(correction.value!, correction.exact!, 100000), '−0.00001');
});

test('a failed observation resolves only when an exact cumulative read arrives', () => {
  const data = reply([leaf({ spent: [{ knownAt: 148, totalRaw: '700' }],
    unknown: [{ knownAt: 124, reason: 'readFailed', project: 'p', until: 148 }] })]);
  const missing = spendingItems(data, { from: null, hour: 124, end: 148 })[0];
  assert.equal(missing.unknownValue, true);
  assert.deepEqual(missing.unknown, { readFailed: 1 });
  assert.equal(missing.finalUnknown, false);
  const known = spendingItems(data, { from: 125, hour: 148, end: 148 })[0];
  assert.equal(known.unknownValue, false);
  assert.equal(known.exact, 700n);
});

test('linked output makes the whole unknown, including a later range', () => {
  const data = reply([leaf({ spent: [{ knownAt: 100, totalRaw: '90' }],
    unknown: [{ knownAt: 124, reason: 'linkedOutput', project: 'p', until: null }] })]);
  const items = spendingItems(data, { from: 130, hour: 148, end: 148 });
  assert.equal(items[0].unknownValue, true);
  assert.equal(groupTree(items, { id: 'all', label: 'All' }).valueUnknown, true);
});

test('ownerless output remains named and limits only ranges that include it', () => {
  const data = reply([leaf({ spent: [{ knownAt: 148, totalRaw: '70' }] })]);
  data.unattributed = [{ knownAt: 124, reason: 'ownerUnknown', project: '99', until: 125 }];
  const early = spendingItems(data, { from: null, hour: 100, end: 148 });
  assert.equal(early.at(-1)?.present, false);
  assert.equal(early.at(-1)?.finalUnknown, true);
  const whole = spendingItems(data, { from: null, hour: 148, end: 148 });
  assert.deepEqual(whole.at(-1)?.unknown, { ownerUnknown: 1 });
  assert.equal(groupTree(whole, { id: 'all', label: 'All' }).valueUnknown, true);
  const later = spendingItems(data, { from: 125, hour: 148, end: 148 });
  assert.equal(later.at(-1)?.present, false);
  assert.equal(later.at(-1)?.finalUnknown, false);
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
  assert.match(footnote({ ...data, beforeFirstRead: 2 })!, /Earlier spending is missing for 2 projects/);
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
  const text = { format: (value: number, exact: bigint | null) => formatSpent(value, exact, data.costScale), unit: 'IC', valueLabel: 'Spent' };
  for (const tip of sliceTooltip(other, root, text)) {
    assert.equal(tip.rows[0].value, '0.003 IC');
    assert.match(tooltipText(tip), /State name unavailable/);
    assert.doesNotMatch(tooltipText(tip), /Stored state name|language differs|State [56]:/);
  }
});
