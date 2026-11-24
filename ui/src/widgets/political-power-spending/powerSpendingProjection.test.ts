import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { BreakdownPart, PowerSpendingBreakdownReply } from '../../generated/dashboard';
import { commandSpendingProjection } from './powerSpendingProjection.ts';

const part = (key: string, raw: string | null, extra: Partial<BreakdownPart> = {}): BreakdownPart => ({
  key, name: key, ordinal: 0, other: false, raw, state: raw === null ? 'notRecorded' : 'known', reason: raw === null ? `${key} was not recorded` : null,
  share: null, count: null, last: null, children: [], ...extra,
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
      ],
      unrecordedHours: 4,
    },
    gainedRaw: '134700', lostAtCapRaw: '7600', gainedReason: null, warnings: [],
  };
  const model = commandSpendingProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', reply]]) }), { currency: 'Command' });
  assert.deepEqual(model.rows.map(row => row.key), ['decisions', 'laws']);
  const decisions = model.rows[0]!;
  assert.deepEqual(decisions.cells.spent!.value, { kind: 'known', value: 101000n });
  assert.deepEqual(decisions.cells.last!.value, { kind: 'known', value: 90 });
  assert.equal(decisions.cells.share!.value.kind, 'known');
  const child = decisions.children![0]!;
  assert.equal(child.cells.share!.blank, true, 'a child row leaves the category share empty');
  assert.deepEqual(model.rows[1]!.cells.spent!.value, { kind: 'notRecorded', reason: 'laws was not recorded' });
  assert.equal(model.titleSummary, '1,450 spent · 1,347 gained · 76 lost at the cap');
  assert.equal(model.titleSummaryPartial, 'Some hours were not recorded', 'a partial total carries its hatch and reason');
});
