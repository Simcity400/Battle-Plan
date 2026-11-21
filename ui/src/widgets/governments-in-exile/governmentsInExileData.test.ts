import assert from 'node:assert/strict';
import test from 'node:test';
import { sortRows } from '../../charts/table/logic.ts';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { exileNumbers, exilesAt, exileStructure, exileTableValues, exileTooltip, exileWarnings, legitimacyShare, legitimacyText, manpowerText, readNote,
  validateExiles } from './governmentsInExileData.ts';
import type { ExileRow } from './governmentsInExileData.ts';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';
import type { CountryGroupReply } from '../../generated/dashboard';

const exile = (id: string, tag: string, host: string | null = 'ENG') => ({ id, tag, host });
const reply = (exiles: ReturnType<typeof exile>[]) =>
  ({ branchId: '1', view: 'exiles', exiles, balances: [], current: null, unavailable: [], warnings: [] as string[] });
const at = (wire: ReturnType<typeof validateExiles>, hour = 100) => ({ ...wire, hour });
const all = () => true;
const keyOf = (row: ExileRow) => row.key;

const cells = (rows: Record<string, Partial<Record<'legitimacy' | 'manpower' | 'cap' | 'leaders' | 'deployedAir' | 'readHour', number>>>) =>
  valuesOf(new Map(Object.entries(rows).flatMap(([id, values]) => Object.entries(values).map(([cell, value]) => [liveKey(id, cell), value] as const))),
    'Not recorded at this hour');

test('each exile at the hour is a row by its id with its host, its numbers live cells, a number not recorded missing by name', () => {
  const data = at(validateExiles(reply([exile('7', 'POL'), exile('9', 'NOR', null)])));
  const view = exilesAt(data, all);
  assert.deepEqual(view.rows.map(row => [row.key, row.tag, row.host]), [['7', 'POL', 'ENG'], ['9', 'NOR', null]]);
  assert.equal(view.unknownHosts, 1);
  assert.deepEqual(exileWarnings(data, view), ['1 refused exile host identity: the host has no tag at this hour']);
  const values = cells({ 7: { legitimacy: 50, manpower: 400, cap: 900, leaders: 2, deployedAir: 100, readHour: 90 }, 9: { manpower: 400, cap: 900 } });
  const pol = exileNumbers(values, '7'), nor = exileNumbers(values, '9');
  assert.equal(legitimacyText(pol.legitimacy!), '50.00%');
  assert.equal(legitimacyShare(pol.legitimacy!), 0.5);
  assert.equal(manpowerText(nor), '400 / 900');
  assert.match(readNote(pol, 100)!, /^Exile resources last read /);
  assert.equal(readNote(pol, 90), null);
  assert.equal(nor.legitimacy, null, 'never zero');
  const tip = exileTooltip(nor, 100, { name: 'Norway' }, null);
  assert.deepEqual(tip.rows.find(row => row.label === 'Deployed air crews')?.value, 'Not recorded at this hour');
  const table = exileTableValues(values);
  assert.equal(table.get(liveKey('7', 'air')), 100, 'the Air crews column reads the deployed air cell');
  assert.equal(sortRows(view.rows, row => table.get(liveKey(row.key, 'legitimacy')), 'descending', keyOf)[0].tag, 'POL');
});

test('an exile is kept when it or its host is chosen', () => {
  const data = at(validateExiles(reply([exile('7', 'POL'), exile('8', 'LUX', 'FRA')])));
  assert.deepEqual(exilesAt(data, tag => tag === 'ENG').rows.map(row => row.tag), ['POL']);
  assert.deepEqual(exilesAt(data, tag => tag === 'LUX').rows.map(row => row.tag), ['LUX']);
});

test('the followed reply renders only when its exiles change, never as their numbers move', () => {
  assert.equal(exileStructure(validateExiles(reply([exile('7', 'POL')]))), exileStructure(validateExiles(reply([exile('7', 'POL')]))));
  assert.notEqual(exileStructure(validateExiles(reply([exile('7', 'POL')]))), exileStructure(validateExiles(reply([exile('7', 'POL', 'FRA')]))));
});

test('malformed replies are refused rather than read as no exiles', () => {
  for (const change of [
    (value: ReturnType<typeof reply>) => { (value as Record<string, unknown>).view = 'balances'; },
    (value: ReturnType<typeof reply>) => { (value.exiles[0] as Record<string, unknown>).id = 7; },
    (value: ReturnType<typeof reply>) => { delete (value as Record<string, unknown>).unavailable; },
  ]) { const value = reply([exile('7', 'POL')]); change(value); assert.throws(() => validateExiles(value)); }
});

const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
const gapGroups = (tags: readonly string[], gap = { firstHour: 110, lastHour: 120 }): CountryGroupReply => ({ branchId: '1', warnings: [],
  humans: tags.map(tag => ({ tag, firstHour: 0, lastHour: 200 })), factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: tags.map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: 200, slot, countryId: slot + 1 }] })),
  unobserved: { humans: [gap], factions: [], wars: [] } });
const GAP = 'Group membership is unavailable at this time.';

test('an exile left out where the chosen group’s membership was not recorded makes the hour unknown, never no exiles', () => {
  const groups = gapGroups(['POL', 'ENG']);
  const match = countryMatch(humans, [], groups, 200), unrecorded = (hour: number) => membershipUnrecordedAt(humans, groups, hour);
  const data = (hour: number) => at(validateExiles(reply([exile('7', 'POL')])), hour);
  assert.deepEqual([exilesAt(data(115), match.matches, unrecorded).rows.length, exilesAt(data(115), match.matches, unrecorded).membership], [0, GAP]);
  assert.deepEqual([exilesAt(data(100), match.matches, unrecorded).rows.map(row => row.tag), exilesAt(data(100), match.matches, unrecorded).membership], [['POL'], null]);
});
