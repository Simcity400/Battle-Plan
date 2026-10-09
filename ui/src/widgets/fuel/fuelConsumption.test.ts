import test from 'node:test';
import assert from 'node:assert/strict';
import { NumericReply } from '../../dashboard/numericReply.ts';
import type { FuelConsumptionRead, FuelDemandHistoryReply, FuelDemandReply, FuelDemandStackReply } from '../../generated/dashboard';
import { consumptionAt, consumptionBasis, consumptionCoverage, consumptionForActor, consumptionGeometry, consumptionHistoryNumbers, consumptionMatches,
  consumptionRuns, consumptionSelection, demandNumbers, fuelConsumptionToken, mergeConsumptionWindows, readFuelConsumption,
  consumptionLevelAvailable, consumptionParts, consumptionStackNumbers, fuelConsumptionLevelToken, readFuelConsumptionLevel, siteLevel, stackedConsumptionRuns,
  validateFuelConsumption, validateFuelDemand, validateFuelDemandHistory, validateFuelDemandStack } from './fuelConsumption.ts';

const read: FuelConsumptionRead = { firstHour: 100, lastHour: 123, startsSegment: true, segmentStartHour: 100,
  country: '7', countryInstance: 9, nativeDailyRaw: ['-240000', null, '0'], knownDailyRaw: ['-240000', '0', '0'],
  positiveDailyRaw: ['0', '0', '0'], reasons: [null, 'Synthetic incomplete air census', null] };
const detail: FuelDemandReply = { branchId: '1', tag: 'synthetic country', hour: 100, read, topologyReason: null, warnings: [],
  parts: [{ key: 'native-occurrence', kind: 0, reference: '9007199254741000', parent: '0', ordinal: 0, owners: ['7', '8', '9'],
    wingOwner: null, group: null, topologyReason: null, site: null, amountRaw: '-240000', reason: null, recordedName: '', nameReason: null }],
  commands: [], fleets: [], sites: [{ list: 1, ordinal: 0, base: '9007199254741001', containerOwner: null,
    capacityRaw: null, demilitarized: null, wings: [], reason: 'Synthetic empty refused site' }] };
const history: FuelDemandHistoryReply = { branchId: '1', tag: detail.tag, hour: 100, country: '7', countryInstance: 9,
  segmentStartHour: 100, window: 0, key: 'native-occurrence', firstHour: 0, lastHour: 719, warnings: [], runs: [
    { firstHour: 100, lastHour: 123, startsSegment: true, amountRaw: '-240000', reason: null, recordedName: '', nameReason: null },
    { firstHour: 124, lastHour: 147, startsSegment: false, amountRaw: null, reason: 'Selected occurrence absent', recordedName: null, nameReason: null },
    { firstHour: 148, lastHour: 171, startsSegment: false, amountRaw: '240000', reason: null, recordedName: 'Supplying-read name', nameReason: null },
  ] };

test('current consumption selects signed rows and pins native supplying identity rather than receipt dates', () => {
  assert.equal(validateFuelDemand(detail), detail);
  assert.equal(validateFuelConsumption({ branchId: '1', tag: detail.tag, reads: [read], warnings: [] }).reads[0], read);
  const selection = consumptionSelection(detail, detail.parts[0], 'campaign')!;
  assert.deepEqual(readFuelConsumption(fuelConsumptionToken(selection), 'campaign', '1'), selection);
  assert.equal(readFuelConsumption(fuelConsumptionToken(selection), 'other', '1'), null);
  assert.equal(consumptionForActor(selection, 'campaign', '1', 'other tag'), null);
  assert.equal(consumptionMatches(selection, { ...read, segmentStartHour: 124 }), false);
  assert.equal(consumptionMatches(selection, { ...read, countryInstance: 10 }), false);
  assert.equal(consumptionBasis(detail, { ...read, countryInstance: null }), null);
  assert.throws(() => validateFuelDemand({ ...detail, parts: [...detail.parts, detail.parts[0]] }), /malformed/);
  assert.throws(() => validateFuelConsumption({ branchId: '1', tag: detail.tag, reads: [{ ...read, nativeDailyRaw: ['0', '0', '0'] }], warnings: [] }), /malformed/);
});

test('an occurrence keeps its original basis across a native rename but stops at a real gap or new instance', () => {
  const selection = consumptionSelection({ ...detail, tag: 'X' }, detail.parts[0], 'campaign')!;
  const aliases = new Map([['X', [{ firstHour: 100, lastHour: 123 }]], ['Y', [{ firstHour: 124, lastHour: 200 }]]]);
  const summaries = [{ tag: 'X', branchId: '1', warnings: [], reads: [read, { ...read, firstHour: 124, lastHour: 200, country: '8', countryInstance: 10 }] },
    { tag: 'Y', branchId: '1', warnings: [], reads: [{ ...read, firstHour: 124, lastHour: 147, segmentStartHour: 124 },
      { ...read, firstHour: 150, lastHour: 171, segmentStartHour: 150 }, { ...read, firstHour: 172, lastHour: 200, countryInstance: 10 }] }];
  assert.equal(consumptionForActor(selection, 'campaign', '1', 'Y', aliases), selection);
  assert.deepEqual(consumptionCoverage(summaries, selection, aliases).map(span => [span.firstHour, span.lastHour]), [[100, 123], [124, 147]]);
  assert.equal(consumptionForActor({ ...selection, basis: { ...selection.basis, hour: 150 } }, 'campaign', '1', 'Y', aliases), null,
    'a saved basis from the replacement holder is unavailable');
});

test('the same required-per-day history keeps signed text, positive geometry and missing occurrences as gaps', () => {
  assert.equal(validateFuelDemandHistory(history), history);
  const runs = consumptionRuns([history]);
  assert.equal(consumptionAt(runs, 110)?.amountRaw, '-240000');
  assert.equal(consumptionAt(runs, 124)?.amountRaw, null);
  assert.deepEqual(consumptionGeometry(runs, 100000), [
    { firstHour: 100, lastHour: 123, value: 0, breakBefore: true },
    { firstHour: 148, lastHour: 171, value: 2.4, breakBefore: true },
  ]);
  const carried = { ...history, window: 1, firstHour: 720, lastHour: 1439,
    runs: [{ ...history.runs[2], firstHour: 710, lastHour: 800 }] };
  assert.deepEqual(mergeConsumptionWindows([history, carried], [], [1]), [carried]);
  assert.equal(consumptionRuns([{ ...carried, runs: [{ ...carried.runs[0], lastHour: 719 }] }, carried])[0].lastHour, 800);
});

test('content-only names leave mounted demand and history ownership while real native amount/topology changes replace it', () => {
  const store = new NumericReply<FuelDemandReply>();
  const first = store.update('actor/read', detail, demandNumbers)!;
  assert.equal(store.update('actor/read', undefined, demandNumbers), first);
  assert.equal(store.update('actor/read', { ...detail, warnings: ['New content finding'],
    parts: detail.parts.map(part => ({ ...part, recordedName: 'Changed cached presentation', nameReason: 'New language refusal' })) }, demandNumbers), first);
  assert.equal(first.parts[0].recordedName, null);
  const changed = store.update('actor/read', { ...detail, parts: detail.parts.map(part => ({ ...part, amountRaw: '-240001' })) }, demandNumbers);
  assert.notEqual(changed, first);
  assert.notEqual(store.update('actor/read', { ...detail, sites: [{ ...detail.sites[0], ordinal: 1 }] }, demandNumbers), changed);
  assert.equal(store.update('new native actor', undefined, demandNumbers), undefined);
  const windows = new NumericReply<FuelDemandHistoryReply[]>();
  const old = windows.update('unit/segment', [history], consumptionHistoryNumbers);
  assert.equal(windows.update('unit/segment', [{ ...history, runs: history.runs.map(run => ({ ...run, recordedName: 'New name' })) }], consumptionHistoryNumbers), old);
  assert.equal(windows.update('other unit/segment', undefined, consumptionHistoryNumbers), undefined);
});

test('all/family/command/fleet/site levels select only their native occurrence cohort and retain scoped pins', () => {
  const cohort: FuelDemandReply = { ...detail, parts: [
    { ...detail.parts[0], group: '37' },
    { ...detail.parts[0], key: 'task-force', kind: 2, parent: '41' },
    { ...detail.parts[0], key: 'wing-0', kind: 1, group: null },
    { ...detail.parts[0], key: 'wing-1', kind: 1, group: null },
  ], commands: [{ reference: '37', parent: '0', country: '0', armyGroup: false, members: [detail.parts[0].reference], children: [], recordedName: '', nameReason: null }],
    fleets: [{ reference: '41', recordedName: '', nameReason: null }], sites: [{ ...detail.sites[0], wings: ['wing-0', 'wing-1'] }] };
  assert.equal(consumptionParts(cohort, 'all').length, 4);
  assert.deepEqual(consumptionParts(cohort, 'family:0').map(part => part.key), ['native-occurrence']);
  assert.deepEqual(consumptionParts(cohort, 'command:37').map(part => part.key), ['native-occurrence']);
  assert.deepEqual(consumptionParts(cohort, 'fleet:41').map(part => part.key), ['task-force']);
  const hierarchy = { ...cohort, commands: [...cohort.commands, { ...cohort.commands[0], reference: '38', armyGroup: true,
    members: [detail.parts[0].reference], children: ['37'] }] };
  assert.deepEqual(consumptionParts(hierarchy, 'command:38').map(part => part.key), ['native-occurrence'], 'direct and child membership must not double-count a division');
  const site = siteLevel(cohort.sites[0], 0);
  assert.deepEqual(consumptionParts(cohort, site).map(part => part.key), ['wing-0', 'wing-1']);
  assert.equal(consumptionLevelAvailable({ ...cohort, sites: [{ ...cohort.sites[0], base: '1' }] }, site), false);
  const level = { campaign: 'campaign', branch: '1', tag: cohort.tag, country: '7', countryInstance: 9, segmentStartHour: 100, level: site };
  assert.deepEqual(readFuelConsumptionLevel(fuelConsumptionLevelToken(level), 'campaign', '1'), level);
  assert.equal(readFuelConsumptionLevel(fuelConsumptionLevelToken(level), 'campaign', 'other'), null);
  assert.equal(readFuelConsumptionLevel(fuelConsumptionLevelToken({ ...level, level: 'garbage' }), 'campaign', '1'), null);
});

test('5000 batch series stream once and retain numeric ownership through basis/label churn without dropping repeated keys', () => {
  let visits = 0;
  const count = 5000;
  const parts = Array.from({ length: count }, (_unused, index) => ({ get key() { visits++; return `occurrence:${index}`; }, runs: history.runs }));
  const streamed = stackedConsumptionRuns([{ parts }]);
  assert.equal(streamed.size, count);
  assert.equal(streamed.get('occurrence:4999')![0].amountRaw, '-240000');
  assert.ok(visits <= count * 3, 'building every keyed series must not scan all parts once per key');
  const stack: FuelDemandStackReply = { branchId: history.branchId, tag: history.tag, hour: history.hour, country: history.country,
    countryInstance: history.countryInstance, segmentStartHour: history.segmentStartHour, window: 0, firstHour: 0, lastHour: 719,
    warnings: [], parts: [{ key: history.key, runs: history.runs }] };
  assert.equal(validateFuelDemandStack(stack), stack);
  assert.throws(() => validateFuelDemandStack({ ...stack, parts: [...stack.parts, stack.parts[0]] }), /malformed/);
  const store = new NumericReply<ReturnType<typeof consumptionStackNumbers>>();
  const first = store.update('native cohort', consumptionStackNumbers([stack]), value => value);
  assert.equal(store.update('native cohort', undefined, value => value), first);
  const renamed = { ...stack, hour: 124, warnings: ['New content finding'], parts: stack.parts.map(part => ({ ...part,
    runs: part.runs.map(run => ({ ...run, recordedName: 'Different supplying name' })) })) };
  assert.equal(store.update('native cohort', consumptionStackNumbers([renamed]), value => value), first);
  const changed = { ...stack, parts: stack.parts.map(part => ({ ...part, runs: part.runs.map(run => ({ ...run, lastHour: run.lastHour + 1 })) })) };
  assert.notEqual(store.update('native cohort', consumptionStackNumbers([changed]), value => value), first);
  assert.equal(store.update('other native cohort', undefined, value => value), undefined);
});
