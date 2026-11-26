import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { ArtHandle } from '../../families/plan.ts';
import { formatValueState } from '../../families/valueFormat.ts';
import type { BreakdownAmount, BreakdownPart, PowerSpendingBreakdownReply } from '../../generated/dashboard';
import { commandSpendingProjection } from './powerSpendingProjection.ts';

const exact = (numerator: string, denominator = '1'): BreakdownAmount => ({ numerator, denominator });

const part = (key: string, amount: BreakdownAmount | null, extra: Partial<BreakdownPart> = {}): BreakdownPart => ({
  key, name: key, ordinal: 0, other: false, amount, state: amount === null ? 'notRecorded' : 'known', reason: amount === null ? `${key} was not recorded` : null,
  share: null, parentShare: null, nameMissing: null, count: null, last: null, art: null, children: [], ...extra,
});

const icon = (key: string) => ({ key, frame: 0, text: true });

test('spending rows are the product\'s categories and items, dated by their latest payment, with the title figures the rows do not show', () => {
  const reply: PowerSpendingBreakdownReply = {
    from: 0, hour: 100, through: 120,
    breakdown: {
      total: { amount: exact('1450'), state: 'partial', reason: 'Some hours were not recorded' },
      parts: [
        part('decisions', exact('1010'), { share: 0.7, last: 90, art: icon('decision_icon_small'), children: [part('war', exact('25'), { share: 0.02, last: 90 })] }),
        part('laws', null, { art: icon('change_government') }),
        part('ideas:mod_category', exact('20'), { share: 11 / 5975, art: icon('pol_idea') }),
        part('modSpend', exact('10'), { share: 0 }),
      ],
      unrecordedHours: 4,
    },
    gained: exact('1347'), lostAtCap: exact('76'), gainedReason: null, lostAtCapReason: null, warnings: [],
  };
  const asked: ArtHandle[] = [];
  const model = commandSpendingProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', reply]]), onArtDemand: handle => asked.push(handle) }), { currency: 'Command' });
  assert.deepEqual(model.rows.map(row => row.key), ['decisions', 'laws', 'ideas:mod_category', 'modSpend']);
  const decisions = model.rows[0]!;
  assert.deepEqual(decisions.cells.spent!.value, { kind: 'known', value: 101000000n }, 'at the power scale');
  assert.deepEqual(decisions.cells.last!.value, { kind: 'known', value: 90 });
  assert.equal(decisions.cells.share!.value.kind, 'known');
  const share = (row: number) => {
    const shown = formatValueState(model.rows[row]!.cells.share!.value, model.columns.find(column => column.key === 'share')!.format);
    return 'text' in shown ? `${shown.kind} ${shown.text}` : shown.kind;
  };
  assert.equal(share(0), 'known 70%');
  assert.equal(share(2), 'known 0.1%', 'a share above zero never prints 0%');
  assert.equal(share(3), 'zero 0%');
  assert.deepEqual(model.rows.map(row => row.cells.picture!.iconSlot === true), [true, true, true, false], 'a category the product names no picture for shows its row with none');
  assert.deepEqual(asked.map(handle => handle.kind === 'sprite' && handle.text === true ? handle.key : null), ['decision_icon_small', 'change_government', 'pol_idea']);
  const child = decisions.children![0]!;
  assert.equal(child.cells.picture, undefined, 'items carry no category picture');
  assert.equal(child.cells.share!.blank, true, 'a child row leaves the category share empty');
  assert.deepEqual(model.rows[1]!.cells.spent!.value, { kind: 'notRecorded', reason: 'laws was not recorded' });
  assert.equal(model.titleSummary, '1,450 spent · 1,347 gained · 76 lost at the cap');
  assert.equal(model.titleSummaryPartial, 'Some hours were not recorded', 'a partial total carries its hatch and reason');
  const unnamed = commandSpendingProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', { ...reply, breakdown: { ...reply.breakdown, parts: [part('group_a_cat', exact('1', '20'), { nameMissing: 'The recorded localisation holds no name for group_a_cat' })] } }]]) }), { currency: 'Command' });
  assert.deepEqual(unnamed.rows[0]!.cells.item, { value: { kind: 'notRecorded', reason: 'The recorded localisation holds no name for group_a_cat' }, missingFigure: 'group_a_cat' }, 'a key the localisation does not name is shown with the missing mark and its reason');
  const partlyLost = commandSpendingProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', { ...reply, lostAtCapReason: 'Some losses at the cap were not recorded.' }]]) }), { currency: 'Command' });
  assert.equal(partlyLost.titleSummaryPartial, 'Some hours were not recorded; Some losses at the cap were not recorded.', 'a partly known loss at the cap says why');
  const partlyGained = commandSpendingProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', { ...reply, gainedReason: 'Gains in hours when power changes were not read are not counted.' }]]) }), { currency: 'Command' });
  assert.equal(partlyGained.titleSummaryPartial, 'Some hours were not recorded; Gains in hours when power changes were not read are not counted.', 'a partial gain names why beside its figure');
});
