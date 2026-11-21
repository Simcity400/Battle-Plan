import assert from 'node:assert/strict';
import { test } from 'node:test';
import { occupiedTable, type OccupiedTableContext } from './occupiedTable.ts';
import { cellText } from '../../draw/tables/surface.ts';
import type { OccupiedState, OccupiedStatesReply } from './occupiedStatesData';

const state = (id: number, occupier: string | null, occupied: string | null): OccupiedState => ({
  state: id, name: `State ${id}`, nameMissing: null, owner: occupied, occupier, occupied, garrisonRequiredSign: 1,
  garrisonAlert: false, assignedKnown: false, garrisonMissing: null, law: 'mod_law', lawMissing: null });
const reply = (states: OccupiedState[], extra: Partial<OccupiedStatesReply> = {}): OccupiedStatesReply => ({ states, complete: true, missing: null,
  laws: [{ token: 'mod_law', name: 'Moderate', icon: null, missing: null }], strengthMissing: 'Strength needs inputs', garrisonRedRaw: '30000', warnings: [], ...extra });
const context = (tags: string[] | null): OccupiedTableContext => ({ filter: { passing: tags && Object.fromEntries(tags.map(tag => [tag, [[0, 100]] as const])), unrecorded: [], bounds: [0, 101] }, role: 'occupier',
  labels: { GER: [{ from: 0, name: 'German Reich', flag: 'ger.png' }], POL: [{ from: 0, name: 'Poland' }] }, lawAs: 'dot' });

test('the rows built from a reply are its states the chosen countries occupy, each named, and say why when none is', () => {
  const states = [state(1, 'GER', 'POL'), state(2, 'SOV', 'POL'), state(3, null, 'POL')];
  const all = occupiedTable(reply(states), context(null), 5);
  assert.deepEqual(all.rows.map(row => row.key), ['1', '2', '3']);
  assert.equal(all.empty, null);
  const german = occupiedTable(reply(states), context(['GER']), 5);
  assert.deepEqual(german.rows.map(row => row.key), ['1'], 'a state no country names cannot be matched');
  assert.deepEqual(german.rows[0]!.cells.occupier, { looks: [{ from: 0, name: 'German Reich', flag: 'ger.png' }], iconSlot: true, country: 'GER' });
  assert.deepEqual(occupiedTable(reply(states), context(['SOV']), 5).rows[0]!.cells.occupier, { missing: 'Country name unavailable', iconSlot: true, country: 'SOV' },
    'a country whose name is not held says so, never a guess');
  assert.equal(occupiedTable(reply(states.slice(0, 2)), context(['ITA']), 5).empty, 'No occupied states match these countries at this moment.');
  assert.equal(occupiedTable(reply(states), context(['ITA']), 5).empty, 'Occupied states unavailable for these countries.', 'an unnamed occupier makes none matching unknown');
  const unread = occupiedTable(reply(states, { missing: 'not read' }), context(null), 5);
  assert.deepEqual([unread.rows.length, unread.empty], [0, 'Occupied states weren’t recorded at this time: not read'], 'not recorded: no row, the reason');
});

test('a country passes only within its ranges, and a state left out is unknown where membership was not recorded', () => {
  const states = [state(1, 'GER', 'POL')];
  const filter = { passing: { GER: [[10, 20]] as const }, unrecorded: [[30, 40, 'Membership not recorded']] as const, bounds: [10, 21, 30, 41] };
  const at = (hour: number) => occupiedTable(reply(states), { ...context(null), filter }, hour);
  assert.deepEqual(at(15).rows.map(row => row.key), ['1']);
  assert.equal(at(25).empty, 'No occupied states match these countries at this moment.');
  assert.equal(at(35).empty, 'Membership not recorded');
});

test('a country cell carries its looks, for the worker to name it as it was at the hour drawn', () => {
  const looks = { GER: [{ from: 0, name: 'Germany', flag: 'old.png' }, { from: 20, name: 'German Reich', flag: 'new.png' }] };
  const cell = occupiedTable(reply([state(1, 'GER', 'POL')]), { ...context(null), labels: looks }, 10).rows[0]!.cells.occupier;
  assert.equal(cell.looks, looks.GER);
  assert.deepEqual([cellText(cell, null, 10), cellText(cell, null, 25)], ['Germany', 'German Reich']);
});
