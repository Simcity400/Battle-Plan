import test from 'node:test';
import assert from 'node:assert/strict';
import { consumptionLevelAvailable, consumptionParts, demandGeometry, levelComplete, validateFuelDemand, validateFuelDemandHistory } from './fuelConsumption.ts';
import { detailWindows, fuelRecipientFor, joinedRuns, readBasis } from './fuelDrill.ts';
import type { FuelDemandPart, FuelDemandReply } from '../../generated/dashboard';

const basis = { branchId: '1', tag: 'ONE', hour: 48, country: 7, countryInstance: 0 };

test("a pick follows its country to another tag only where the pick's country spans name it", () => {
  const pick = { campaign: 'c', branch: '1', basis };
  assert.equal(fuelRecipientFor(pick, 'c', '1', 'ONE'), pick);
  assert.equal(fuelRecipientFor(pick, 'c', '1', 'TWO'), null);
  const aliases = new Map([['ONE', [{ firstHour: 0, lastHour: 99 }]], ['TWO', [{ firstHour: 100, lastHour: 200 }]]]);
  assert.equal(fuelRecipientFor(pick, 'c', '1', 'TWO', aliases), pick);
  assert.equal(fuelRecipientFor(pick, 'c', '1', 'TRE', aliases), null);
  assert.equal(readBasis('1', 'ONE', { readHour: 48, country: 7, countryInstance: null }), null, 'no basis before the instance is known');
});

test('a detail history asks the windows its view and its moment reach, and joins a read two windows repeat', () => {
  assert.deepEqual(detailWindows([0, 5000], [700, 1500], 3000, 720), [0, 1, 2, 4]);
  assert.deepEqual(detailWindows([800, 5000], [0, 100], null, 720), []);
  assert.deepEqual(joinedRuns([[{ firstHour: 700, lastHour: 719 }], [{ firstHour: 700, lastHour: 723 }, { firstHour: 724, lastHour: 747 }]]),
    [{ firstHour: 700, lastHour: 723 }, { firstHour: 724, lastHour: 747 }]);
});

const part = (key: string, kind: number, patch: Partial<FuelDemandPart> = {}): FuelDemandPart => ({ key, kind, reference: key, parent: '0', ordinal: 0, owners: null,
  wingOwner: null, group: null, topologyReason: null, site: null, amountRaw: '100', reason: null, recordedName: key, nameReason: null, ...patch });
const demand = (): FuelDemandReply => ({ tag: 'ONE', asked: 'ONE', readHour: 48, country: 7, countryInstance: 0,
  consumption: { nativeDailyRaw: ['1', '2', null], knownDailyRaw: ['1', '2', '0'], positiveDailyRaw: ['1', '2', '0'], reasons: [null, null, 'navy unread'] },
  parts: [part('d1', 0, { group: '50' }), part('d2', 0, { reference: '61' }), part('d3', 0), part('w1', 1), part('t1', 2, { parent: '80' }), part('t2', 2, { amountRaw: null, reason: 'rate unread' })],
  commands: [{ reference: '50', parent: '0', country: 7, armyGroup: true, members: [], children: ['60'], recordedName: 'Group', nameReason: null },
    { reference: '60', parent: '50', country: 7, armyGroup: false, members: ['61'], children: [], recordedName: 'Army', nameReason: null }],
  fleets: [{ reference: '80', recordedName: 'Fleet', nameReason: null }],
  sites: [{ list: 0, ordinal: 3, base: '90', containerOwner: 7, capacityRaw: null, demilitarized: null, reason: null, wings: ['w1'] }],
  topologyReason: null, warnings: [] });

test("a level shows the consumers the pack's own links put under it, and its sum is a subtotal when any is unknown", () => {
  const reply = validateFuelDemand(demand());
  const keys = (level: string) => consumptionParts(reply, level).map(item => item.key);
  assert.deepEqual(keys('command:50'), ['d1', 'd2'], "an army group reaches its armies' members");
  assert.deepEqual(keys('fleet:80'), ['t1']);
  assert.deepEqual(keys('family:2'), ['t1', 't2']);
  const site = `site:${JSON.stringify([0, 0, 3, '90', 7])}`;
  assert.deepEqual(keys(site), ['w1']);
  assert.equal(consumptionLevelAvailable(reply, 'fleet:81'), false);
  assert.equal(levelComplete(reply, 'family:0', consumptionParts(reply, 'family:0')), true);
  assert.equal(levelComplete(reply, 'family:2', consumptionParts(reply, 'family:2')), false, 'a rate not read');
  assert.equal(levelComplete(reply, 'all', consumptionParts(reply, 'all')), false);
});

test("a consumer's history is drawn only where a read held it, a new line after each gap", () => {
  const history = { tag: 'ONE', country: 7, window: 0, firstHour: 0, lastHour: 719, warnings: [], parts: [{ key: 'd1', runs: [
    { firstHour: 0, lastHour: 23, startsSegment: true, amountRaw: '200000', reason: null },
    { firstHour: 24, lastHour: 47, startsSegment: false, amountRaw: null, reason: 'not in the pack' },
    { firstHour: 48, lastHour: 71, startsSegment: false, amountRaw: '100000', reason: null }] }] };
  const runs = validateFuelDemandHistory(history).parts[0].runs;
  assert.deepEqual(demandGeometry(runs, 100000), [{ firstHour: 0, lastHour: 23, value: 2, breakBefore: true }, { firstHour: 48, lastHour: 71, value: 1, breakBefore: true }]);
});
