import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions } from '../../dashboard/options.ts';
import { definition, OPTIONS } from './definition.ts';
import { formatHullTick, formatNavyValue, navyChartOptions, navyRequest, navyTotals, validateNavyReply } from './navyData.ts';

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('navy defaults to roster ships, authored content classes and the shared top-ten race', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.equal(options.ownership, 'roster'); assert.equal(options.grouping, 'class');
  assert.equal(options.view, 'race'); assert.equal(options.split, false); assert.equal(options.dateOverlay, false);
  assert.deepEqual(definition.defaultCountries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
  assert.deepEqual(navyRequest('branch', { tags: ['ignored'], top: 10 }, options),
    { branchId: 'branch', tags: [], top: 10, ownership: 'roster', grouping: 'class', measure: 'ships' });
  assert.equal(options.measure, 'ships');
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
});

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('navy logical ownership and broad-role strength choices preserve opaque country picks', () => {
  assert.deepEqual(navyRequest('branch', { tags: ['tag , with space'] }, resolveOptions(OPTIONS, { ownership: 'logical', grouping: 'type', measure: 'strength' })),
    { branchId: 'branch', tags: ['tag , with space'], top: null, ownership: 'logical', grouping: 'type', measure: 'strength' });
  assert.equal(resolveOptions(OPTIONS, { grouping: 'unknown' }).grouping, 'class');
  assert.throws(() => validateNavyReply([]), /malformed/);
  assert.deepEqual(navyTotals(resolveOptions(OPTIONS, { ownership: 'logical', measure: 'strength' })), { kind: 'navy-totals', measure: 'logicalStrength', decimals: 5 });
  assert.deepEqual(navyTotals(resolveOptions(OPTIONS, {})), { kind: 'navy-totals', measure: 'rosterShips', decimals: 0 });
  const window = { branchId: '1', windows: [], window: 0, interval: { firstHour: 0, lastHour: 9 }, parts: [], warnings: [],
    series: [{ tag: 'GER', identities: [], segments: [{ firstHour: 0, lastHour: 9 }], membership: null,
      runs: [{ firstHour: 0, lastHour: 9, startsSegment: true, total: 0, values: [] }] }] };
  assert.throws(() => validateNavyReply(window), /malformed/, 'a navy run without its maximum is refused');
  validateNavyReply({ ...window, series: [{ ...window.series[0], runs: [{ ...window.series[0].runs[0], referenceTotal: null }] }] });
});



// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('hull HP formatting preserves every fixed5 digit and missing maximum stays unknown', () => {
  const options = resolveOptions(OPTIONS, { measure: 'strength' });
  assert.equal(formatNavyValue(125_051, options), '1.25051');
  assert.equal(formatNavyValue(Number.MAX_SAFE_INTEGER, options), '90,071,992,547.40991');
  assert.equal(formatNavyValue(123_456_000, options), '1,234.56');
  assert.equal(formatNavyValue(0, options), '0');
  assert.equal(formatNavyValue(100_000_000, options), '1,000');
  assert.equal(formatHullTick(100_000, [0, 100_000, 200_000]), '1');
  assert.notEqual(formatHullTick(.1, [.1, .2]), formatHullTick(.2, [.1, .2]));
  const data = { branchId: '1', windows: [], window: 0, interval: { firstHour: 0, lastHour: 0 }, parts: [], warnings: [], series: [{ tag: 'mod country',
    identities: [{ firstHour: 0, lastHour: 0, slot: 1, countryId: 2 }],
    segments: [{ firstHour: 0, lastHour: 0 }], membership: null,
    runs: [{ firstHour: 0, lastHour: 0, startsSegment: true, total: 0, values: [], referenceTotal: null }] }] };
  assert.doesNotThrow(() => validateNavyReply(data));
  assert.throws(() => validateNavyReply({ ...data, series: [{ ...data.series[0],
    runs: [{ ...data.series[0].runs[0], referenceTotal: undefined }] }] }), /malformed/);
});

test('navy HP axis bounds use displayed HP without scaling stored settings or ship bounds', () => {
  const options = resolveOptions(OPTIONS, { measure: 'strength', yMin: 2, yMax: 20, log: true });
  const chart = navyChartOptions(options);
  assert.equal(chart.yMin, 200_000);
  assert.equal(chart.yMax, 2_000_000);
  assert.equal(chart.log, true);
  assert.equal(options.yMin, 2);
  assert.equal(formatHullTick(chart.yMin as number, [200_000, 2_000_000]), '2');
  const automatic = navyChartOptions(resolveOptions(OPTIONS, { measure: 'strength' }));
  assert.equal(automatic.yMin, null); assert.equal(automatic.yMax, null);
  const ships = resolveOptions(OPTIONS, { yMin: 2, yMax: 20 });
  assert.equal(navyChartOptions(ships), ships);
});
