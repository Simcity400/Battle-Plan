import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey } from '../../charts/live.ts';
import { valuesOf } from '../../test-support/liveValues.ts';
import { manpowerKey, manpowerTotalKey } from '../../dashboard/factionDetails.ts';
import { manpowerRows, manpowerTableValues, poolHeading, poolsOffered, shareOf, shownPool } from './poolsData.ts';
import { chosenMembers } from './manpowerLine.ts';
import type { FactionDetailReply, FactionManpowerMember } from '../../generated/dashboard';

const reply = (extra: Partial<FactionDetailReply> = {}): FactionDetailReply => ({
  branchId: '1', faction: '9', logo: null, definitions: [], manifest: null, goals: [], completed: [], cancelled: [], goalReason: null,
  resources: [], manpower: [{ ordinal: 0, tag: 'AAA' }, { ordinal: 1, tag: null }, { ordinal: 2, tag: 'AAA' }], manpowerUnlocked: true, researchUnlocked: false,
  researchCountries: ['AAA'], doctrineFolders: [], refusals: [], warnings: [], ...extra,
});

test('only the pools the hour supplies are offered: a locked or unread one never shows as empty', () => {
  assert.deepEqual(poolsOffered(reply()), ['resources', 'manpower']);
  assert.deepEqual(poolsOffered(reply({ resources: null, manpowerUnlocked: false, researchUnlocked: null, doctrineFolders: [{ ordinal: 0, token: 'x', name: null, reason: 'n' }] })),
    ['research', 'doctrine']);
  assert.equal(shownPool(['resources', 'manpower'], 'manpower'), 'manpower');
  assert.equal(shownPool(['resources'], 'manpower'), 'resources');
  assert.equal(shownPool([], 'manpower'), null);
});

test('each manpower occurrence is its own row; amounts, shares and the total come from the live cells', () => {
  const rows = manpowerRows(reply());
  assert.deepEqual(rows.map(row => [row.key, row.tag]), [['manpower:0', 'AAA'], ['manpower:1', null], ['manpower:2', 'AAA']]);
  const values = valuesOf(new Map([[manpowerKey(0), 300], [manpowerKey(2), 100], [manpowerTotalKey, 400]]), 'Not read yet');
  const table = manpowerTableValues(values);
  assert.equal(table.get(liveKey('manpower:0', 'manpower')), 300);
  assert.equal(table.get(liveKey('manpower:2', 'share')), 0.25);
  assert.equal(table.get(liveKey('manpower:1', 'manpower')), undefined);
  assert.equal(table.missing(liveKey('manpower:1', 'manpower')), 'Not read yet');
  assert.equal(poolHeading(reply(), 'manpower', values), '400 manpower');
  assert.equal(poolHeading(reply(), 'manpower', valuesOf(new Map(), 'Not read yet')), 'Total not recorded');
  assert.equal(shareOf(100, 400), '25.0%');
  assert.equal(shareOf(100, 0), '—');
});

test('a picked member is its holder at the hour picked, every occurrence of it; a later holder of the tag never joins', () => {
  const member = (key: string, tag: string, slot: number | null, countryId: number, first: number, last: number): FactionManpowerMember =>
    ({ key, tag, slot, countryId, runs: [{ firstHour: first, lastHour: last, value: 1, startsSegment: true }] });
  const members = [member('0:3:7:AAA', 'AAA', 3, 7, 0, 99), member('1:3:7:AAA', 'AAA', 3, 7, 50, 99), member('0:4:8:AAA', 'AAA', 4, 8, 100, 200), member('0::9:AAA', 'AAA', null, 9, 0, 200)];
  assert.deepEqual(chosenMembers(members, 'AAA', 60).map(item => item.key), ['0:3:7:AAA', '1:3:7:AAA']);
  assert.deepEqual(chosenMembers(members, 'AAA', 150).map(item => item.key), ['0:4:8:AAA']);
});
