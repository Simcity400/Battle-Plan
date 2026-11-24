import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { emptyReason, garrisonBar, garrisonCell, garrisonText, liveRaw, occupiedRows, percentCell, strengthBar, validateOccupiedStates } from './occupiedStatesData.ts';
import type { OccupiedState, OccupiedStatesReply } from './occupiedStatesData';

const state = (id: number, occupier: string | null, occupied: string | null, extra: Partial<OccupiedState> = {}): OccupiedState => ({
  state: id, name: `State ${id}`, nameMissing: null, owner: 'OWN', occupier, occupied, garrisonRequiredSign: 1,
  garrisonAlert: false, assignedKnown: false, garrisonMissing: 'Assigned manpower is incomplete',
  law: 'mod_law', lawMissing: null, ...extra,
});
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
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL', { garrisonRequiredSign: -1 })), { bar: 0n, red: true });
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL', { garrisonRequiredSign: 0 })), { bar: null, red: false });
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL', { garrisonRequiredSign: null })), { bar: null, red: false });
  assert.deepEqual(garrisonCell(state(1, 'GER', 'POL')), { bar: 'cell', red: false });
  assert.equal(garrisonBar(50000), 50000n);
  assert.equal(garrisonBar(-50000), -50000n);
  assert.deepEqual([garrisonText(-1), garrisonText(0), garrisonText(250000), garrisonText(123456)], ['!', '-', '2.50', '1.23']);
  const values = valuesOf(new Map([[liveKey('1', 'resistance'), 2500000]]), 'Not recorded at this hour');
  assert.equal(liveRaw(values, '1', 'resistance'), '2500000');
  assert.equal(liveRaw(values, '1', 'resistanceSpeed'), null);
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
  assert.equal(emptyReason({ ...reply([state(1, 'GER', 'POL')]), missing: 'not read' }, view([{}])), 'Occupied states weren’t recorded at this time: not read');
});

import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveTableTooltip, resolveTableRows } from '../../families/table/model.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';
import { occupiedStatesProjection } from './occupiedProjection.ts';

test('missing garrison and unresolved manpower report notRecorded with source reason, never notApplicable, and tooltip keeps exact bigint precision', () => {
  const missingState = state(1, 'GER', 'POL', {
    garrisonRequiredSign: null,
    assignedKnown: false,
    garrisonMissing: 'No state garrisons row at this hour',
  });
  const bigState = state(2, 'GER', 'POL', {
    garrisonRequiredSign: 1,
    assignedKnown: true,
    garrisonMissing: null,
  });
  const rep = reply([missingState, bigState]);
  const big = 9007199254740993n;
  const live = {
    get: (k: string) => (k === liveKey('2', 'requiredManpower') || k === liveKey('2', 'assignedManpower') ? Number(big) : undefined),
    raw: (k: string) => (k === liveKey('2', 'requiredManpower') || k === liveKey('2', 'assignedManpower') ? big : undefined),
    missing: () => 'Not a live cell of this reply',
  };
  const model = occupiedStatesProjection.project(
    createBoundedReadView({ hour: 50, panels: new Map([['main', rep]]), live: new Map([['main', live]]) }),
    { filter: { passing: null, unrecorded: [], bounds: [] }, role: 'occupier', labels: { GER: [{ from: 0, name: 'Germany' }], POL: [{ from: 0, name: 'Poland' }] }, lawAs: 'dot', filterAll: true },
  );
  const s1 = model.rows.find(r => r.key === '1')!;
  assert.deepEqual(s1.cells.garrison!.value, { kind: 'notRecorded', reason: 'No state garrisons row at this hour' });
  const settings = tableSettingsOf(undefined, COLUMNS, TABLE_BASE);
  const flat = resolveTableRows(model, settings);
  const tip1 = deriveTableTooltip(flat, '1', model.columns)!;
  assert.equal(tip1.rows.find(r => r.label === 'Required manpower')?.valueState?.kind, 'missing');
  assert.equal(tip1.rows.find(r => r.label === 'Required manpower')?.detail, 'No state garrisons row at this hour');
  assert.equal(tip1.rows.find(r => r.label === 'Assigned manpower')?.valueState?.kind, 'missing');
  assert.equal(tip1.rows.find(r => r.label === 'Assigned manpower')?.detail, 'No state garrisons row at this hour');

  const tip2 = deriveTableTooltip(flat, '2', model.columns)!;
  assert.equal(tip2.rows.find(r => r.label === 'Required manpower')?.valueState?.text, '9,007,199,254,740,993');
  assert.equal(tip2.rows.find(r => r.label === 'Assigned manpower')?.valueState?.text, '9,007,199,254,740,993');
});
