import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import type { AirAce, AirAcesReply } from '../../generated/dashboard';
import { acesProjection } from './acesProjection.ts';
import { validateAces } from './acesData.ts';

const ace = (id: string, tag: string | null, extra: Partial<AirAce> = {}): AirAce => ({
  id, tag, name: `Ace ${id}`, callsign: 'Bubi', wing: null, wingName: null, aircraft: null, alive: true, firstSeen: 61000000, death: null,
  portrait: null, portraitMissing: 'Ace portrait culture not recorded', ...extra,
});

const reply = (aces: AirAce[]): AirAcesReply => ({ readHour: 61000010, missing: null, aces, warnings: [] });

const filter = (passing: Record<string, [number, number][]> | null) => ({ filter: { passing, unrecorded: [], bounds: [] } });

const project = (raw: unknown, options = filter(null)) =>
  acesProjection.project(createBoundedReadView({ hour: 61000020, panels: new Map([['main', raw]]) }), options);

test('an ace shows its wing, when it became an ace, and its portrait as missing with the resolver’s reason', () => {
  const model = project(reply([ace('1', 'GER', { wing: '5', wingName: 'Jagdgeschwader 52' }), ace('2', 'CHI', { firstSeen: null })]));
  const [first, second] = model.rows;
  assert.deepEqual(first!.cells.wing!.value, knownValue('Jagdgeschwader 52'));
  assert.equal(first!.cells.since!.value.kind, 'known');
  assert.deepEqual([first!.cells.portrait!.artShape, first!.cells.portrait!.icon, first!.cells.ace!.subtext], ['portrait', undefined, '“Bubi”']);
  assert.deepEqual(first!.cells.portrait!.value, notRecorded('Ace portrait culture not recorded'));
  assert.deepEqual(second!.cells.wing!.value, knownValue('No wing'));
  assert.deepEqual(second!.cells.since!.value, notRecorded('First sighting not recorded'));
});

test('the country filter keeps only aces of countries passing at the hour; an ace with no recorded country cannot pass', () => {
  const model = project(reply([ace('1', 'GER'), ace('2', 'CHI'), ace('3', null)]), filter({ GER: [[0, 70000000]] }));
  assert.deepEqual(model.rows.map(row => row.key), ['1']);
});

test('the roster is refused without the portrait pair', () => {
  const { portrait: _portrait, ...old } = ace('1', 'GER');
  assert.throws(() => validateAces(reply([old as AirAce])));
  assert.throws(() => validateAces(reply([ace('1', 'GER', { portrait: null, portraitMissing: null })])));
});
