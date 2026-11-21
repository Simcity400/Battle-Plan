import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { NO_ROW, NOT_RECORDED } from '../../dashboard/follow/liveCells.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { affiliation, affiliationEpochs, breakdownItems, epochOf, rowAmount, validateBreakdownReply, withFinals } from './breakdownData.ts';
import type { TerritoryBreakdownReply } from './types';

const reply: TerritoryBreakdownReply = {
  measure: 'states', rule: 'owned',
  regions: [{ id: 7, name: 'North', missing: null }, { id: 8, name: null, missing: 'localisation missing' }],
  rows: [
    { key: 'ger7', tag: 'GER', slot: 1, region: 7 },
    { key: 'ita8', tag: 'ITA', slot: 2, region: 8 },
    { key: 'none', tag: null, slot: null, region: null },
    { key: 'sov7', tag: 'SOV', slot: 3, region: 7 },
    { key: 'gone', tag: 'FRA', slot: 4, region: 7 },
  ],
  missing: null, areaReason: null, warnings: [],
};

const cells = new Map([
  [liveKey('ger7', 'value'), 5], [liveKey('ger7', 'denominator'), 3],
  [liveKey('none', 'value'), 1], [liveKey('none', 'denominator'), 1],
  [liveKey('sov7', 'value'), 0], [liveKey('sov7', 'denominator'), 1],
]);
const values = valuesOf(cells, key => key.startsWith('gone') ? NO_ROW : NOT_RECORDED);

const groups = { factions: [{ id: '9', firstHour: 0, lastHour: 100, members: [{ tag: 'GER', firstHour: 20, lastHour: 100 }], incomplete: [] }],
  unobserved: { humans: [], factions: [], wars: [] } } as unknown as CountryGroupReply;

const looks = {
  country: (tag: string) => ({ name: `Country ${tag}`, flag: undefined, flagPending: false, flagReason: undefined }),
  faction: () => 'Axis', colour: (id: string) => id === '9' ? '#aa0000' : undefined, selected: () => true,
};

test('a row\'s states are its cells\' exact fraction, the other measures whole; a row gone is none, a cell not read is unknown', () => {
  assert.deepEqual(rowAmount(values, 'ger7', 'states'), { value: 5 / 3, exact: 5n, denominator: 3n });
  assert.deepEqual(rowAmount(valuesOf(new Map([[liveKey('ger7', 'value'), 40]]), NOT_RECORDED), 'ger7', 'area'), { value: 40, exact: 40n, denominator: 1n });
  assert.equal(rowAmount(values, 'ita8', 'provinces'), null, 'not recorded is no amount, never zero');
  assert.equal(rowAmount(valuesOf(new Map([[liveKey('ger7', 'value'), 5]]), NOT_RECORDED), 'ger7', 'states'), null, 'states without their denominator are not known');
  assert.deepEqual(rowAmount(values, 'gone', 'states'), { value: 0, exact: 0n, denominator: 1n }, 'the table holds no row of the key any more: it holds nothing');
});

test('rows become faction, country and region slices of their cells; a holding not known is unknown, none is left out', () => {
  const items = breakdownItems(reply, values, 'states', affiliation(groups, 30), looks);
  assert.deepEqual(items.map(item => [item.path.map(key => key.label), item.value, item.unknownValue ?? false]), [
    [['Axis', 'Country GER', 'North'], 5 / 3, false],
    [['Non-aligned', 'Country ITA', 'Region 8'], null, true],
    [['Unassigned territory', 'Unassigned territory', 'Unknown region'], 1, false],
  ]);
  assert.equal(items[0].path[0].colour, '#aa0000');
  assert.equal(items[0].exactDenominator, 3n);
  assert.equal(items[2].exactDenominator, undefined, 'a whole state has no denominator to show');
  const chosen = breakdownItems(reply, values, 'states', affiliation(groups, 30), { ...looks, selected: tag => tag === 'ITA' });
  assert.deepEqual(chosen.map(item => item.path[1].label), ['Country ITA']);
});

test('the reply holds rows and names only; one carrying amounts in its rows is the old shape and refused', () => {
  assert.equal(validateBreakdownReply(reply), reply);
  const old = { ...reply, rows: [{ tag: 'GER', slot: 1, region: 7, controlled: { provinces: 1, victoryPoints: 0, area: null, states: [] }, owned: null }], countries: [] };
  assert.throws(() => validateBreakdownReply(old), /malformed/);
});

test('a faction is not known while memberships were not read or a country is in two', () => {
  const unread = { ...groups, unobserved: { humans: [], factions: [{ firstHour: 5, lastHour: 25 }], wars: [] } } as unknown as CountryGroupReply;
  assert.equal(affiliation(unread, 22)('GER').unknown, true);
  assert.equal(affiliation(unread, 30)('GER').faction?.id, '9');
  const twice = { ...groups, factions: [...groups.factions, { ...groups.factions[0], id: '10' }] } as unknown as CountryGroupReply;
  assert.deepEqual(affiliation(twice, 30)('GER'), { faction: undefined, unknown: true });
});

test('hours in one stretch between memberships\' edges are affiliated alike, so the slices are made again only past an edge', () => {
  const epochs = affiliationEpochs(groups);
  assert.deepEqual(epochs, [0, 20, 101]);
  assert.equal(epochOf(epochs, 21), epochOf(epochs, 99));
  assert.notEqual(epochOf(epochs, 19), epochOf(epochs, 20), 'GER joins at 20');
  assert.notEqual(epochOf(epochs, 100), epochOf(epochs, 101), 'the faction ends after 100');
  for (const hour of [-5, 0, 19, 20, 50, 100, 101, 400]) {
    const same = [hour, ...epochs].filter(other => epochOf(epochs, other) === epochOf(epochs, hour));
    for (const other of same) assert.deepEqual(affiliation(groups, other)('GER'), affiliation(groups, hour)('GER'), `${hour} and ${other}`);
  }
});

test('Fill gives each slice its amount at the view’s end, lists what only the end holds at none now, and never takes an unknown end for zero', () => {
  const key = (id: string) => ({ id, label: id });
  const now = [{ path: [key('axis'), key('GER')], value: 3, exact: 3n }, { path: [key('axis'), key('ITA')], value: 2, exact: 2n }];
  const end = [{ path: [key('axis'), key('GER')], value: 5, exact: 5n }, { path: [key('allies'), key('ENG')], value: 4, exact: 4n },
    { path: [key('axis'), key('ITA')], value: null, exact: null, unknownValue: true }];
  const filled = withFinals(now, end);
  assert.deepEqual(filled.map(item => [item.path.map(part => part.id).join('/'), item.value, item.final ?? null, item.finalUnknown ?? false]), [
    ['axis/GER', 3, 5, false], ['axis/ITA', 2, null, true], ['allies/ENG', 0, 4, false]]);
  assert.equal(withFinals(now, [])[1].final, 0, 'held now and not at the end: it empties');
});
