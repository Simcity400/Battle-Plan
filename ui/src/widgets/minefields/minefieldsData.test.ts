import test from 'node:test';
import assert from 'node:assert/strict';
import { minesAt, minesText, orderRegions, ownerAmount, ownerSeries, refusedOwnersAt, regionHolders, regionRuns, validateMinefields, validateReports } from './minefieldsData.ts';

const reply = validateMinefields({
  window: { firstHour: 0, lastHour: 20, known: [{ firstHour: 0, lastHour: 10 }], reads: [[0, 10, 0]], series: [
    { name: 'GER@7|5', runs: [[0, 10, '300000', true]] },
    { name: 'GER@7|extent', runs: [[0, 10, '40', true]] },
    { name: 'ENG@3|5', runs: [[0, 10, '100000', true]] },
    { name: 'ENG@3|50', runs: [[0, 10, '-100000', true]] },
  ] },
  regions: [{ id: '5', name: 'North Sea' }, { id: '50', name: 'Far Sea' }], scale: 100000, warnings: [],
});

test('a region below the witnessed extent is a proven zero; beyond it or without a witness is unknown', () => {
  const owners = ownerSeries(reply.window);
  assert.equal(ownerAmount(owners.get('GER@7')!, '6', 2), 0n);
  assert.equal(ownerAmount(owners.get('GER@7')!, '45', 2), null);
  assert.equal(ownerAmount(owners.get('ENG@3')!, '6', 2), null);
});

test('regions sum positive owners on one scale, keep negatives exact and count unknown owners', () => {
  const owners = ownerSeries(reply.window);
  const at = minesAt(reply, owners, 3)!;
  const north = at.regions.find(region => region.region === '5')!;
  assert.equal(north.total, 400000n);
  assert.deepEqual(north.owners.map(owner => owner.tag), ['GER', 'ENG']);
  const far = at.regions.find(region => region.region === '50')!;
  assert.equal(far.total, 0n);
  assert.equal(far.unknownOwners, 1);
  assert.deepEqual(orderRegions(at.regions, 'mines').map(region => region.region), ['5', '50']);
  assert.equal(minesAt(reply, owners, 15), null);
  assert.equal(minesText(150000n, 100000), '1.5');
  assert.equal(minesText(-100000n, 100000), '−1');
});

test('reports keep their order and carry no strike date', () => {
  const reports = validateReports({ hour: 3, readHour: 3, region: 5, warnings: [], reports: [
    { kind: 1, ordinal: 0, victim: 'ENG@3', mineOwner: 'GER@7', ship: 'Synthetic', equipment: null, damage: '25000', sunk: false, firstSeen: null },
    { kind: 1, ordinal: 1, victim: 'ENG@3', mineOwner: 'GER@7', ship: 'Synthetic', equipment: null, damage: '25000', sunk: false, firstSeen: 2 }] });
  assert.deepEqual(reports.reports.map(report => report.ordinal), [0, 1]);
  assert.throws(() => validateReports({ ...reports, reports: [{ kind: 1 }] }));
});

test('a refused owner vector, named or not, leaves the inventory incomplete rather than empty', () => {
  const refused = validateMinefields({
    window: { firstHour: 0, lastHour: 10, known: [{ firstHour: 0, lastHour: 10 }], reads: [[0, 10, 0]], series: [
      { name: 'GER@7|5', runs: [[0, 10, '300000', true]] },
      { name: 'GER@7|extent', runs: [[0, 10, '40', true]] },
      { name: 'ITA@8|refused', runs: [[0, 4, '1', true]] },
      { name: '?|refused', runs: [[0, 10, '2', true]] },
    ] },
    regions: [{ id: '5', name: 'North Sea' }], scale: 100000, warnings: [],
  });
  const owners = ownerSeries(refused.window);
  assert.deepEqual([...owners.keys()], ['GER@7', 'ITA@8']);
  assert.equal(refusedOwnersAt(refused.window, owners, 2), 3);
  assert.equal(refusedOwnersAt(refused.window, owners, 6), 2);
  const at = minesAt(refused, owners, 2)!;
  assert.equal(at.refused, 3);
  // ITA (refused, no extent) and the two unnamed owners are unknown in the North Sea.
  assert.equal(at.regions[0].unknownOwners, 3);
});

test('the reports reply carries the region as the backend number', () => {
  assert.throws(() => validateReports({ hour: 3, readHour: 3, region: '5', warnings: [], reports: [] }), /malformed/u);
});

test("a region's drill runs keep witnessed zeros and leave refused hours unknown", () => {
  const history = validateMinefields({
    window: { firstHour: 0, lastHour: 20, known: [{ firstHour: 0, lastHour: 20 }], reads: [[0, 20, 0]], series: [
      { name: 'GER@7|5', runs: [[2, 4, '300000', false], [8, 9, '100000', false]] },
      { name: 'GER@7|extent', runs: [[0, 11, '40', true], [16, 20, '3', true]] },
      { name: 'GER@7|refused', runs: [[12, 15, '1', true]] },
    ] },
    regions: [{ id: '5', name: 'North Sea' }], scale: 100000, warnings: [],
  }).window;
  assert.deepEqual(regionRuns(ownerSeries(history).get('GER@7')!, '5'), [
    [0, 1, '0', true], [2, 4, '300000', false], [5, 7, '0', false], [8, 9, '100000', false], [10, 11, '0', false]]);
});

test('an owner with nothing at the hour has no minefields then, not unknown ones', () => {
  const history = validateMinefields({
    window: { firstHour: 0, lastHour: 9, known: [{ firstHour: 0, lastHour: 9 }], reads: [[0, 9, 0]], series: [
      { name: 'GER@7|extent', runs: [[0, 4, '40', true]] },
      { name: 'ENG@3|5', runs: [[5, 9, '100000', true]] },
      { name: 'ENG@3|extent', runs: [[5, 9, '40', true]] },
    ] },
    regions: [{ id: '5', name: 'North Sea' }], scale: 100000, warnings: [],
  });
  const owners = ownerSeries(history.window);
  const north = minesAt(history, owners, 7)!.regions.find(region => region.region === '5')!;
  assert.deepEqual(north.owners.map(owner => owner.owner), ['ENG@3']);
  assert.equal(north.unknownOwners, 0, 'GER@7 stopped laying mines at 4');
});

test('two holders of a reused tag stay two holders in a region, each at an hour it held its mines', () => {
  const reused = validateMinefields({
    window: { firstHour: 0, lastHour: 20, known: [{ firstHour: 0, lastHour: 20 }], reads: [[0, 20, 0]], series: [
      { name: 'GER@7|5', runs: [[0, 8, '100000', true]] },
      { name: 'GER@9|5', runs: [[12, 20, '200000', true]] },
      { name: 'ENG@3|50', runs: [[0, 20, '100000', true]] },
    ] },
    regions: [{ id: '5', name: 'North Sea' }], scale: 100000, warnings: [],
  });
  assert.deepEqual(regionHolders(ownerSeries(reused.window), '5'), [{ key: 'GER@7', tag: 'GER', hour: 8 }, { key: 'GER@9', tag: 'GER', hour: 20 }]);
});
