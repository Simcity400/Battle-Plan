import assert from 'node:assert/strict';
import test from 'node:test';
import { damagedGroupLabel, factoryGroupPresentation, factoryMeasure, groupCount, partRows, typeChoices, validateFactoryGroups, validateFactoryParts } from './factoriesData.ts';
import type { FactoryPartsReply } from '../../generated/dashboard';

const part = (group: string, name: string, value: number) => ({ group, part: name, value });
const REPLY: FactoryPartsReply = { hour: 7, tag: 'GER', unexplained: ['civilian'], missing: null, warnings: [], parts: [
  part('military', 'total', 12), part('military', 'damaged', 1), part('military', 'toOverlord', 2), part('military', 'toOthers', 1),
  part('civilian', 'total', 30), part('civilian', 'damaged', 1), part('civilian', 'toOthers', 1), part('civilian', 'consumerGoods', 5),
  part('civilian', 'contractPayments', 1), part('civilian', 'specialProjects', 2), part('civilian', 'licenseImports', 1), part('civilian', 'tradeImports', 3),
] };


test('a group is counted from its parts as the declared count reads them, and a group not recorded is not zero', () => {
  assert.deepEqual(['built', 'topBar', 'free'].map(mode => groupCount(REPLY.parts, 'military', mode as 'free')), [12, 8, 8]);
  assert.deepEqual(['built', 'topBar', 'free'].map(mode => groupCount(REPLY.parts, 'civilian', mode as 'free')), [30, 28, 16]);
  assert.equal(groupCount(REPLY.parts, 'naval', 'free'), null);
});

test('a country’s parts are its chosen groups’ counts, their nonzero parts and an unexplained withholding by name', () => {
  const names = validateFactoryGroups({ missing: null, warnings: [], groups: [
    { key: 'military', flag: 'military_production', buildings: [{ token: 'ordnance_works', name: 'Ordnance Works' }] },
    { key: 'naval', flag: 'naval_production', buildings: [] },
    { key: 'civilian', flag: 'general_production', buildings: [{ token: 'civic_hall', name: 'Civic Workshop' }] },
  ] });
  const rows = partRows(REPLY, ['civilian'], 'free', names);
  assert.deepEqual(rows[0], { label: 'Civic Workshop', value: '16', strong: true, divider: true });
  assert.ok(rows.some(row => row.label === 'Consumer goods' && row.value === '5'));
  assert.equal(rows.at(-1)?.label, 'Unavailable');
  assert.ok(!rows.some(row => row.label === 'Ordnance Works'));
  const missingRows = partRows(REPLY, ['naval'], 'free', names);
  assert.equal(missingRows[0]?.label, 'Name not recorded');
  assert.equal(missingRows[0]?.value, 'Not recorded');
  assert.match(missingRows[0]?.detail ?? '', /naval_production/);
  const unavailableNames = validateFactoryGroups({ missing: 'Content unavailable', warnings: [], groups: [] });
  const unrecordedRows = partRows(REPLY, ['military'], 'free', unavailableNames);
  assert.equal(unrecordedRows[0]?.label, 'Name not recorded');
  assert.equal(unrecordedRows[0]?.value, '8');
  assert.equal(unrecordedRows[0]?.detail, 'Content unavailable');
  assert.deepEqual(partRows({ ...REPLY, missing: 'CountryFactories held no row' }, [], 'free', names), [{ label: 'Not recorded', value: 'CountryFactories held no row', tone: 'muted' }]);
  assert.throws(() => validateFactoryParts({ ...REPLY, parts: [{ group: 'military', part: 'total', value: 1.5 }] }));
});

test('a group is named by the content’s building types, unnamed ones by that, and Name not recorded when none is recorded', () => {
  const names = validateFactoryGroups({ missing: null, warnings: [], groups: [
    { key: 'military', flag: 'military_production', buildings: [{ token: 'ordnance_works', name: 'Ordnance Works' }, { token: 'mod_foundry', name: null }] },
    { key: 'naval', flag: 'naval_production', buildings: [] },
    { key: 'civilian', flag: 'general_production', buildings: [{ token: 'mod_plant', name: null }] },
  ] });
  assert.equal(factoryGroupPresentation('military', names).label, 'Ordnance Works · Building name unavailable');
  assert.equal(factoryGroupPresentation('naval', names).label, 'Name not recorded');
  assert.deepEqual(factoryGroupPresentation('civilian', names), {
    key: 'civilian',
    label: 'Building name unavailable',
    missing: 'Building name not recorded',
    colorIndex: 2,
  });
  assert.equal(damagedGroupLabel(factoryGroupPresentation('civilian', names)), 'Damaged Building name unavailable');
  assert.equal(damagedGroupLabel(factoryGroupPresentation('naval', names)), 'Damaged (Name not recorded)');
  assert.equal(factoryGroupPresentation('civilian', undefined).label, 'Name not recorded');
  assert.deepEqual(typeChoices(names), [
    { value: 'military', label: 'Ordnance Works · Building name unavailable' },
    { value: 'naval', label: 'Name not recorded', hint: 'No building in the recorded content has naval_production' },
    { value: 'civilian', label: 'Building name unavailable', hint: 'Building name not recorded' },
  ]);
  assert.throws(() => validateFactoryGroups({ groups: [{ key: 'military' }], missing: null, warnings: [] }));
});

test('factoryMeasure encodes mode and selected group bitmask', () => {
  assert.equal(factoryMeasure('topBar', ['civilian', 'military']), 'topBar5');
  assert.equal(factoryMeasure('free', []), 'free7');
  assert.equal(factoryMeasure('built', ['naval']), 'built2');
});