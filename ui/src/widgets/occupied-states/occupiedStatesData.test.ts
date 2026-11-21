import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { emptyReason, occupiedDetail, garrisonBar, garrisonCell, garrisonText, garrisonTone, liveRaw, NOT_NAMED, occupationTooltip, occupationTooltipKeys, occupiedRows, percentCell, strengthBar, validateOccupiedStates } from './occupiedStatesData.ts';
import type { OccupiedState, OccupiedStatesReply } from './occupiedStatesData';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { panelStructure } from '../../draw/panels/views.ts';

const state = (id: number, occupier: string | null, occupied: string | null, extra: Partial<OccupiedState> = {}): OccupiedState => ({
  state: id, name: `State ${id}`, nameMissing: null, owner: 'OWN', occupier, occupied, garrisonRequiredSign: 1,
  garrisonAlert: false, assignedKnown: false, garrisonMissing: 'Assigned manpower is incomplete',
  law: 'mod_law', lawMissing: null, ...extra });
const reply = (states: OccupiedState[]): OccupiedStatesReply => ({ states, complete: true, missing: null, laws: [], strengthMissing: 'Strength needs inputs', garrisonRedRaw: '30000', warnings: [] });

test('the reply is checked whole: states ascend once each and every field has its type', () => {
  assert.equal(validateOccupiedStates(reply([state(1, 'GER', 'POL'), state(2, 'GER', 'CZE')])).states.length, 2);
  assert.throws(() => validateOccupiedStates(reply([state(2, 'GER', 'POL'), state(1, 'GER', 'CZE')])));
  assert.throws(() => validateOccupiedStates(reply([state(1, 'GER', 'POL', { garrisonRequiredSign: 2 })])));
  const { garrisonRequiredSign: _sign, assignedKnown: _known, ...old } = state(1, 'GER', 'POL');
  assert.throws(() => validateOccupiedStates(reply([{ ...old, garrisonRequiredRaw: '250000', requiredManpower: 100, assignedManpower: 50 } as unknown as OccupiedState])),
    'the old reply, carrying the moving numbers, is refused');
  assert.throws(() => validateOccupiedStates(reply([{ ...state(1, 'GER', 'POL'), garrisonStrengthRaw: '50000', garrisonRed: false } as unknown as OccupiedState])),
    'the reply carrying the strength in its structure is refused: it is a live cell');
  assert.throws(() => validateOccupiedStates({ ...reply([]), garrisonRedRaw: 0.3 }));
  assert.equal(validateOccupiedStates({ ...reply([]), garrisonRedRaw: null }).garrisonRedRaw, null, 'a content saying no red threshold');
  assert.throws(() => validateOccupiedStates({ ...reply([]), missing: 3 }));
});

test('the worker tells the page only of what the page renders itself, never which country holds what', () => {
  const shape = (states: OccupiedState[], warnings: string[] = []) => panelStructure('occupied-states', { ...reply(states), warnings }, 0);
  const before = shape([state(1, 'GER', 'POL'), state(2, 'ITA', 'GRE')]);
  assert.equal(shape([state(1, 'ITA', 'POL'), state(2, 'GER', 'GRE')]), before);
  assert.equal(shape([state(1, 'GER', 'POL'), state(2, 'HUN', 'GRE')]), before, 'a new country: the worker names every country the recording holds');
  assert.notEqual(shape([state(1, 'GER', 'POL'), state(2, null, 'GRE')]), before, 'a state no country holds');
  assert.notEqual(shape([state(1, 'GER', 'POL'), state(2, 'ITA', 'GRE')], ['Laws not recorded']), before, 'a warning');
  assert.throws(() => panelStructure('occupied-states', { states: 3 }, 0), /malformed/);
});

test('a drawn state’s detail is the state and its law in the reply drawn', () => {
  const drawn = { ...reply([state(1, 'GER', 'POL'), state(2, 'ITA', 'GRE', { law: 'other' })]), laws: [{ token: 'other', name: 'Other law', icon: null, missing: null }] };
  assert.deepEqual(occupiedDetail(drawn, '2'), { value: drawn.states[1], law: drawn.laws[0], strengthMissing: 'Strength needs inputs' });
  assert.equal(occupiedDetail(drawn, '1')?.law, null, 'a law the reply does not list');
  assert.equal(occupiedDetail(drawn, '3'), null);
});

test('a filter matches the occupier or the occupied country, and a state no country names cannot be matched', () => {
  const states = reply([state(1, 'GER', 'POL'), state(2, 'ITA', 'GRE'), state(3, null, 'YUG')]);
  assert.deepEqual(occupiedRows(states, null, 'occupier').rows.map(row => row.state), [1, 2, 3]);
  const byOccupier = occupiedRows(states, tag => tag === 'GER', 'occupier');
  assert.deepEqual([byOccupier.rows.map(row => row.state), byOccupier.selectionUnknown], [[1], true]);
  const byOccupied = occupiedRows(states, tag => tag === 'GRE', 'occupied');
  assert.deepEqual([byOccupied.rows.map(row => row.state), byOccupied.selectionUnknown], [[2], false]);
});

test('cells round half away from zero and clamp; the bar narrows the strength as the game does', () => {
  assert.equal(percentCell('9950000'), '100%');
  assert.equal(percentCell('12000000'), '100%');
  assert.equal(percentCell('-1'), '0%');
  assert.equal(percentCell('4449999'), '44%');
  assert.equal(strengthBar('50000'), 50000n);
  assert.equal(strengthBar('-50000'), -50000n);
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL', { garrisonRequiredSign: -1 })), { bar: 0n, red: true }, 'unmet: an empty bar and red, whatever the strength');
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL', { garrisonRequiredSign: 0 })), { bar: null, red: false });
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL', { garrisonRequiredSign: null })), { bar: null, red: false }, 'no garrison row, no bar');
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL')), { bar: 'cell', red: false }, 'required: the bar and its red are the strength’s live cell');
  assert.equal(garrisonBar(50000), 50000n);
  assert.equal(garrisonBar(-50000), -50000n);
  const tone = garrisonTone('30000');
  assert.deepEqual([tone(29999), tone(30000), tone(100000)], ['occupation-garrison-alert', undefined, undefined], 'red below the content’s threshold');
  assert.equal(garrisonTone(null)(0), undefined, 'no threshold, never red');
  assert.deepEqual([garrisonText(-1), garrisonText(0), garrisonText(250000), garrisonText(123456)], ['!', '-', '2.50', '1.23'], 'the painted amount as main drew it');
});

test('the tooltip reads the live values and names why one is missing, and a strength not established says why', () => {
  const values = valuesOf(new Map([[liveKey('1', 'resistance'), 2500000], [liveKey('1', 'compliance'), 0], [liveKey('1', 'requiredManpower'), 1200],
    [liveKey('1', 'garrison'), 250000], [liveKey('2', 'assignedManpower'), 900]]), 'Not recorded at this hour');
  assert.equal(liveRaw(values, '1', 'resistance'), '2500000');
  assert.equal(liveRaw(values, '1', 'resistanceSpeed'), null);
  const row = occupiedRows(reply([state(1, 'GER', 'POL')]), null, 'occupier').rows[0];
  const tip = occupationTooltip(row, values, tag => `Name ${tag}`, 'Strength needs inputs', token => `Law ${token}`);
  const value = (label: string) => tip.rows.find(item => item.label === label)?.value;
  assert.deepEqual([value('Owner'), value('Occupier'), value('Occupation law')], ['Name OWN', 'Name GER', 'Law mod_law'],
    'every column the table can hide into the tooltip is in it');
  const unknown = occupationTooltip(occupiedRows(reply([state(9, 'GER', 'POL', { owner: null, law: null, lawMissing: 'No law row' })]), null, 'occupier').rows[0],
    values, tag => tag, '', token => token);
  assert.deepEqual([unknown.rows.find(item => item.label === 'Owner')?.value, unknown.rows.find(item => item.label === 'Occupation law')?.value],
    ['Unknown country', 'No law row'], 'missing owner and law say so');
  assert.equal(value('Resistance'), '25%');
  assert.equal(value('Resistance speed'), 'Not recorded at this hour');
  const tone = (label: string) => tip.rows.find(item => item.label === label)?.tone;
  assert.deepEqual([tone('Resistance speed'), tone('Resistance'), tone('Garrison strength')], ['muted', undefined, 'muted'], 'the family panel: a missing value is muted in the value place');
  assert.equal(value('Compliance'), '0%');
  assert.equal(value('Garrison strength'), 'Assigned manpower is incomplete');
  const strong = valuesOf(new Map([[liveKey('5', 'garrisonStrength'), 87500]]), NOT_NAMED);
  const strength = (id: number, extra: Partial<OccupiedState>, values = strong) => occupationTooltip(occupiedRows(reply([state(id, 'GER', 'POL', extra)]), null, 'occupier').rows[0],
    values, tag => tag, 'Strength needs inputs', token => token).rows.find(item => item.label === 'Garrison strength')?.value;
  assert.equal(strength(5, { garrisonMissing: null }), '87.5%', 'the strength from its live cell');
  assert.equal(strength(6, { garrisonMissing: null }), 'Strength needs inputs', 'no cell named and no reason: the reply’s general reason');
  assert.equal(strength(6, { garrisonMissing: null }, valuesOf(new Map(), 'Not recorded at this hour')), 'Not recorded at this hour', 'a cell named but not recorded at the hour shown says so');
  assert.equal(value('Occupied country'), 'Name POL');
  assert.deepEqual([value('Required manpower'), value('Required garrison amount'), value('Assigned manpower')], ['1,200', '2.5', '—'], 'garrison numbers from cells; assigned not known');
  const unnamed = valuesOf(new Map([[liveKey('2', 'assignedManpower'), 900]]), NOT_NAMED);
  const known = (id: number) => occupiedRows(reply([state(id, 'GER', 'POL', { assignedKnown: true })]), null, 'occupier').rows[0];
  const assigned = (id: number) => occupationTooltip(known(id), unnamed, tag => tag, '', token => token).rows.find(item => item.label === 'Assigned manpower')?.value;
  assert.deepEqual([assigned(2), assigned(3)], ['900', '0'], 'the summed cell, and zero where no row is assigned and so no cell is named');
  const noGarrison = occupationTooltip(occupiedRows(reply([state(4, 'GER', 'POL', { garrisonRequiredSign: null })]), null, 'occupier').rows[0], values, tag => tag, '', token => token);
  assert.equal(noGarrison.rows.find(item => item.label === 'Required manpower')?.value, '—', 'no garrison row, no garrison number');

  const read = new Set<string>();
  const recording = { get: (key: string) => { read.add(key); return undefined; }, missing: (key: string) => { read.add(key); return NOT_NAMED; } };
  occupationTooltip(known(7), recording, tag => tag, '', token => token);
  assert.deepEqual([...read].filter(key => !occupationTooltipKeys('7').includes(key)), []);
});

const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
const gapGroups = (tags: readonly string[], gap = { firstHour: 110, lastHour: 120 }): CountryGroupReply => ({ branchId: '1', warnings: [],
  humans: tags.map(tag => ({ tag, firstHour: 0, lastHour: 200 })), factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: tags.map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: 200, slot, countryId: slot + 1 }] })),
  unobserved: { humans: [gap], factions: [], wars: [] } });
const GAP = 'Group membership is unavailable at this time.';

test('a state left out where the chosen group’s membership was not recorded makes the hour unknown, never no occupied states', () => {
  const groups = gapGroups(['GER']);
  const match = countryMatch(humans, [], groups, 200);
  const view = (hour: number) => occupiedRows(reply([state(1, 'GER', 'POL')]), tag => match.matches(tag, hour), 'occupier', membershipUnrecordedAt(humans, groups, hour));
  assert.deepEqual([view(115).rows.length, view(115).membership], [0, GAP]);
  assert.deepEqual([view(100).rows.map(row => row.state), view(100).membership], [[1], null]);
});

test('the table says why it shows no rows at the hour, and nothing while it shows some', () => {
  const view = (rows: unknown[], extra: { selectionUnknown?: boolean; membership?: string | null } = {}) => ({ rows, selectionUnknown: false, membership: null, ...extra });
  assert.equal(emptyReason(reply([state(1, 'GER', 'POL')]), view([{}])), null);
  assert.equal(emptyReason(reply([]), view([])), 'No occupied states match these countries at this moment.');
  assert.equal(emptyReason({ ...reply([]), complete: false }, view([])), 'Some occupied states are missing.');
  assert.equal(emptyReason(reply([]), view([], { selectionUnknown: true })), 'Occupied states unavailable for these countries.');
  assert.equal(emptyReason(reply([]), view([], { membership: 'Membership not recorded' })), 'Membership not recorded');
  assert.equal(emptyReason({ ...reply([state(1, 'GER', 'POL')]), missing: 'not read' }, view([{}])), 'Occupied states weren’t recorded at this time: not read', 'not recorded wins over rows');
});
