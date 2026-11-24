import assert from 'node:assert/strict';
import test from 'node:test';
import { factoryMeasure, groupCount, groupName, partRows, typeChoices, validateFactoryGroups, validateFactoryParts } from './factoriesData.ts';
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
  const rows = partRows(REPLY, ['civilian'], 'free');
  assert.deepEqual(rows[0], { label: 'Civilian factories', value: '16', strong: true, divider: true });
  assert.ok(rows.some(row => row.label === 'Consumer goods' && row.value === '5'));
  assert.equal(rows.at(-1)?.label, 'Unavailable');
  assert.ok(!rows.some(row => row.label === 'Military factories'));
  assert.deepEqual(partRows({ ...REPLY, missing: 'CountryFactories held no row' }, [], 'free'), [{ label: 'Not recorded', value: 'CountryFactories held no row', tone: 'muted' }]);
  assert.throws(() => validateFactoryParts({ ...REPLY, parts: [{ group: 'military', part: 'total', value: 1.5 }] }));
});

test('a group is named by the content’s building types, unnamed ones by that, the game’s group where the content names none', () => {
  const names = validateFactoryGroups({ missing: null, warnings: [], groups: [
    { key: 'military', flag: 'military_production', buildings: [{ token: 'arms_factory', name: 'Military Factory' }, { token: 'mod_foundry', name: null }] },
    { key: 'naval', flag: 'naval_production', buildings: [] },
  ] });
  assert.equal(groupName('military', names), 'Military Factory · Building name unavailable');
  assert.equal(groupName('naval', names), 'Naval dockyards');
  assert.equal(groupName('civilian', undefined), 'Civilian factories');
  assert.deepEqual(typeChoices(names).map(choice => choice.value), ['military', 'naval', 'civilian']);
  assert.throws(() => validateFactoryGroups({ groups: [{ key: 'military' }], missing: null, warnings: [] }));
});

test('factoryMeasure encodes mode and selected group bitmask', () => {
  assert.equal(factoryMeasure('topBar', ['civilian', 'military']), 'topBar5');
  assert.equal(factoryMeasure('free', []), 'free7');
  assert.equal(factoryMeasure('built', ['naval']), 'built2');
});