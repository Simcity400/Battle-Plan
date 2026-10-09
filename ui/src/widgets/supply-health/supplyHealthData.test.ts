import assert from 'node:assert/strict';
import test from 'node:test';
import type { SupplyHealthReply } from '../../generated/dashboard';
import { resolveCountries } from '../../dashboard/groups.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { createWidget } from '../../dashboard/model.ts';
import { widgetDefinition } from '../../dashboard/definitions.ts';
import '../definitions.ts';
import { formatShare, supplyDrillNumbers, supplyDrillRuns, supplyShareRuns, supplyTooltip, totalDivisions, validateSupplyHealthReply } from './supplyHealthData.ts';
import { shareStack } from '../../charts/share.ts';
import { clipToSpans, runAt } from '../../charts/series.ts';

const run = { firstHour: 100, lastHour: 102, startsSegment: true, noSupply: 1, strained: 1, inSupply: 1 };
const reply: SupplyHealthReply = { branchId: '1', available: true, reason: null, missingInputs: [],
  series: [{ tag: 'opaque country', runs: [run, { ...run, firstHour: 104, lastHour: 105 }] }], warnings: [] };

test('supply health accepts native counts and explicit unavailable recordings, refusing malformed data', () => {
  assert.equal(validateSupplyHealthReply(reply), reply);
  const missing = { ...reply, available: false, reason: 'DivisionLandInputs is not in this recording.',
    missingInputs: ['DivisionLandInputs'], series: [{ tag: 'opaque country', runs: [] }] };
  assert.equal(validateSupplyHealthReply(missing), missing);
  for (const invalid of [
    { ...reply, available: null }, { ...missing, missingInputs: [] }, { ...missing, series: reply.series },
    { ...reply, reason: 'unexpected' }, { ...reply, missingInputs: ['unexpected'] },
    { ...reply, series: [{ tag: 'opaque', runs: [{ ...run, noSupply: -1 }] }] },
    { ...reply, series: [{ tag: 'opaque', runs: [{ ...run, strained: .5 }] }] },
    { ...reply, series: [{ tag: 'opaque', runs: [{ ...run, inSupply: null }] }] },
    { ...reply, series: [{ tag: 'opaque', runs: [run, run] }] },
    { ...reply, series: [reply.series[0], reply.series[0]] },
    null, [],
  ]) assert.throws(() => validateSupplyHealthReply(invalid), /supply health reply/);
});

test('supply health uses the shared frame, numeric uncapped country convention, and one default country', () => {
  const meta = widgetDefinition('supply-health');
  assert.ok(meta);
  assert.equal(meta.data, true);
  assert.equal(meta.history, true);
  assert.equal(meta.countries?.max, Number.MAX_SAFE_INTEGER);
  assert.equal(meta.countries?.ranked, undefined);
  const tags = Array.from({ length: 5_000 }, (_, index) => `opaque country ${index}`);
  const chosen = resolveCountries({ kind: 'tags', tags }, [], undefined, 100, meta.countries!.max);
  assert.equal(chosen.tags.length, 5_000);
  assert.equal(chosen.dropped, 0);
  const widget = createWidget('supply-health', 'main', 'supply');
  assert.deepEqual(widget.countries, { kind: 'selected' });
  assert.deepEqual(widget.time, { mode: 'campaign' });
});

test('supply shares and tooltip retain exact counts, gaps, zero denominators, and hovered identities', () => {
  assert.equal(formatShare(1, 3), '33.33%');
  assert.equal(formatShare(2, 3), '66.67%');
  assert.equal(formatShare(0, 0), 'Not known');
  const shares = supplyShareRuns(reply.series[0].runs);
  assert.deepEqual(shares[0].counts, [1, 1, 1]);
  const model = shareStack(shares, 3);
  assert.equal(model.edges[model.hours.indexOf(103)], null);
  const tip = supplyTooltip('hover date', 'hovered name', { flag: 'hovered flag' }, run, ['bad', 'warn', 'good']);
  assert.equal(tip.title, 'hover date · hovered name');
  assert.equal(tip.countryFlag?.flag, 'hovered flag');
  assert.deepEqual(tip.rows.map(row => [row.label, row.value, row.swatch]), [
    ['In supply', '33.33% · 1 division', 'good'], ['Strained', '33.33% · 1 division', 'warn'], ['No supply', '33.33% · 1 division', 'bad'],
  ]);
  assert.match(tip.note!, /low received supply still covered by grace/);
  assert.match(tip.note!, /very-low frame 3/);
  const plural = supplyTooltip('date', 'name', undefined, { ...run, strained: 2 }, ['bad', 'warn', 'good']);
  assert.equal(plural.rows[1].value, '50.00% · 2 divisions');
  assert.equal(supplyTooltip('date', 'name', undefined, null, []).rows[0].value, 'Not known');
  const zero = { ...run, noSupply: 0, strained: 0, inSupply: 0 };
  assert.equal(supplyTooltip('date', 'name', undefined, zero, []).rows[0].value, '0');
  assert.match(supplyTooltip('date', 'name', undefined, zero, []).note!, /shares are undefined/);
  const missing = { flagReason: 'Supplying read refused the native flag', flagPending: false };
  assert.deepEqual(supplyTooltip('date', 'name', missing, zero, []).countryFlag, missing);
  assert.deepEqual(supplyTooltip('date', 'name', missing, null, []).countryFlag, missing);
  assert.equal(supplyTooltip('date', 'Country identity not recorded', undefined, null, []).countryFlag, undefined);
});

test('recorded smart-group membership clips the stack and tooltip to its own hours', () => {
  const runs = clipToSpans(reply.series[0].runs, [{ firstHour: 101, lastHour: 104 }]);
  assert.equal(runAt(runs, 100), null);
  assert.equal(runAt(runs, 101)?.noSupply, 1);
  assert.equal(runAt(runs, 103), null);
  assert.equal(runAt(runs, 105), null);
  const model = shareStack(supplyShareRuns(runs), 3);
  assert.equal(model.hours[0], 101);
  assert.equal(model.edges[model.hours.indexOf(103)], null);
});


test('Supply drill follows the selected native identity across tags and preserves unknown gaps', () => {
  const identities = [{ firstHour: 100, lastHour: 102, slot: 1, countryId: 7 }];
  const renamed: SupplyHealthReply = { ...reply, series: [
    { tag: 'A', identities: [...identities, { firstHour: 104, lastHour: 105, slot: 2, countryId: 8 }],
      runs: [run, { ...run, firstHour: 104, lastHour: 105, noSupply: 99 }] },
    { tag: 'B', identities: [{ firstHour: 104, lastHour: 105, slot: 1, countryId: 7 }],
      runs: [{ ...run, firstHour: 104, lastHour: 105, noSupply: 2 }] },
  ] };
  const selected = supplyDrillRuns(renamed, 'A', 100);
  assert.equal(runAt(selected, 103), null);
  assert.equal(runAt(selected, 104)?.noSupply, 2);
  assert.deepEqual(supplyShareRuns(selected).map(run => run.counts), [[1, 1, 1], [2, 1, 1]]);
  assert.equal(totalDivisions(selected[1]), 4);
  assert.equal(formatShare(2, 4), '50.00%');
  assert.equal(supplyDrillRuns(renamed, 'A', 103).length, 0);
});


test('Supply drawn operands ignore warning churn but retain bucket and native identity changes', () => {
  const retained = new NumericReply<SupplyHealthReply>();
  const first = retained.update('same source / selection', reply, supplyDrillNumbers);
  assert.equal(retained.update('same source / selection', undefined, supplyDrillNumbers), first);
  assert.equal(retained.update('same source / selection', { ...reply, warnings: ['renamed content refusal'] }, supplyDrillNumbers), first);
  assert.notDeepEqual(supplyDrillNumbers({ ...reply, series: [{ ...reply.series[0], runs: [{ ...run, noSupply: 2 }] }] }),
    supplyDrillNumbers(reply));
  assert.notDeepEqual(supplyDrillNumbers({ ...reply, series: [{ ...reply.series[0],
    identities: [{ firstHour: 100, lastHour: 102, slot: 1, countryId: 7 }] }] }), supplyDrillNumbers(reply));
});
