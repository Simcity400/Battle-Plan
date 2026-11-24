import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { ArtHandle } from '../../families/plan.ts';
import { formatValueState } from '../../families/valueFormat.ts';
import type { BreakdownPart, PowerSpendingBreakdownReply } from '../../generated/dashboard';
import { commandSpendingProjection } from './powerSpendingProjection.ts';

const part = (key: string, raw: string | null, extra: Partial<BreakdownPart> = {}): BreakdownPart => ({
  key, name: key, ordinal: 0, other: false, raw, state: raw === null ? 'notRecorded' : 'known', reason: raw === null ? `${key} was not recorded` : null,
  share: null, parentShare: null, count: null, last: null, children: [], ...extra,
});

test('spending rows are the product\'s categories and items, dated by their latest payment, with the title figures the rows do not show', () => {
  const reply: PowerSpendingBreakdownReply = {
    from: 0, hour: 100, through: 120,
    breakdown: {
      decimals: 2,
      total: { raw: '145000', state: 'partial', reason: 'Some hours were not recorded' },
      parts: [
        part('decisions', '101000', { share: 0.7, last: 90, children: [part('war', '2500', { share: 0.02, last: 90 })] }),
        part('laws', null),
        part('ideas:mod_category', '2000', { share: 11 / 5975 }),
        part('modSpend', '1000', { share: 0 }),
      ],
      unrecordedHours: 4,
    },
    gainedRaw: '134700', lostAtCapRaw: '7600', gainedReason: null, warnings: [],
  };
  const asked: ArtHandle[] = [];
  const model = commandSpendingProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', reply]]), onArtDemand: handle => asked.push(handle) }), { currency: 'Command' });
  assert.deepEqual(model.rows.map(row => row.key), ['decisions', 'laws', 'ideas:mod_category', 'modSpend']);
  const decisions = model.rows[0]!;
  assert.deepEqual(decisions.cells.spent!.value, { kind: 'known', value: 101000n });
  assert.deepEqual(decisions.cells.last!.value, { kind: 'known', value: 90 });
  assert.equal(decisions.cells.share!.value.kind, 'known');
  const share = (row: number) => {
    const shown = formatValueState(model.rows[row]!.cells.share!.value, model.columns.find(column => column.key === 'share')!.format);
    return 'text' in shown ? `${shown.kind} ${shown.text}` : shown.kind;
  };
  assert.equal(share(0), 'known 70%');
  assert.equal(share(2), 'known 0.2%', 'a share above zero never prints 0%');
  assert.equal(share(3), 'zero 0%');
  assert.deepEqual(model.rows.map(row => row.cells.picture!.iconSlot === true), [true, true, true, false], 'a category the mapping does not know shows its row with no picture');
  assert.deepEqual(asked.map(handle => handle.kind === 'sprite' && handle.text === true ? handle.key : null), ['decision_icon_small', 'change_government', 'pol_idea']);
  const child = decisions.children![0]!;
  assert.equal(child.cells.picture, undefined, 'items carry no category picture');
  assert.equal(child.cells.share!.blank, true, 'a child row leaves the category share empty');
  assert.deepEqual(model.rows[1]!.cells.spent!.value, { kind: 'notRecorded', reason: 'laws was not recorded' });
  assert.equal(model.titleSummary, '1,450 spent · 1,347 gained · 76 lost at the cap');
  assert.equal(model.titleSummaryPartial, 'Some hours were not recorded', 'a partial total carries its hatch and reason');
  const partlyGained = commandSpendingProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', { ...reply, gainedReason: 'Gains in hours when power changes were not read are not counted.' }]]) }), { currency: 'Command' });
  assert.equal(partlyGained.titleSummaryPartial, 'Some hours were not recorded; Gains in hours when power changes were not read are not counted.', 'a partial gain names why beside its figure');
});
