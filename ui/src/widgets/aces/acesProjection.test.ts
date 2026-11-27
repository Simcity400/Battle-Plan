import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveRowTooltip } from '../../families/table/model.ts';
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
  assert.deepEqual(first!.cells.country!.value, { kind: 'pending', reason: 'Name is loading' });
  const tip = deriveRowTooltip({ kind: 'data', key: first!.key, name: first!.name, rank: 1, level: 1, parent: null, childCount: 0, isTreeGroup: false, isOpen: false, isTotal: false, row: first! }, model.columns);
  assert.deepEqual(tip.rows.find(entry => entry.label === 'Country')?.rawValue, { kind: 'pending', reason: 'Name is loading' });
  assert.equal(first!.cells.since!.value.kind, 'known', 'a first sighting shows its date');
  assert.deepEqual([first!.cells.portrait!.artShape, first!.cells.portrait!.art?.status, first!.cells.ace!.subtext], ['portrait', 'unavailable', '“Bubi”']);
  assert.deepEqual(first!.cells.portrait!.value, notRecorded('Ace portrait culture not recorded'));
  assert.deepEqual(second!.cells.wing!.value, knownValue('No wing'));
  assert.deepEqual(second!.cells.since!.value, notRecorded('First sighting not recorded'));
});

test('the country filter keeps only aces of countries passing at the hour; an ace with no recorded country cannot pass', () => {
  const model = project(reply([ace('1', 'GER'), ace('2', 'CHI'), ace('3', null)]), filter({ GER: [[0, 70000000]] }));
  assert.deepEqual(model.rows.map(row => row.key), ['1']);
});

test('the killer’s country is named as it was when the death was seen, not as it is at the displayed hour', () => {
  const asked: (number | undefined)[] = [];
  const identity = {
    resolve(kind: string, key: string, atHour: number | undefined) {
      asked.push(atHour);
      const name = atHour !== undefined && atHour < 61000015 ? 'Japan' : 'Manchukuo';
      return { kind, key, atHour, holder: { status: 'ready', value: key }, tag: { status: 'ready', value: key }, name: { status: 'ready', value: name },
        colour: { status: 'absent', reason: 'none' }, flagChoice: { status: 'absent', reason: 'none' }, validInterval: [0, 1e9], nextChangeAt: Infinity };
    },
  };
  const dead = ace('2', 'CHI', { alive: false, death: { lastAlive: 61000000, byHour: 61000010, seenDead: 61000010, killerAce: '1', killerName: 'Ace 1', killerTag: 'JAP' } });
  const model = acesProjection.project(createBoundedReadView({ hour: 61000020, panels: new Map([['main', reply([dead])]]), identity } as never), filter(null));
  const tip = model.rows[0]!.tooltipExtra;
  const rows = typeof tip === 'function' ? tip() : tip;
  assert.deepEqual(rows?.find(row => row.label === 'Killer country')?.value, knownValue('Japan'));
  assert.ok(asked.includes(61000010));
  const unseen = ace('3', 'CHI', { alive: false, death: { lastAlive: null, byHour: null, seenDead: null, killerAce: null, killerName: null, killerTag: null } });
  const unknown = project(reply([unseen])).rows[0]!.tooltipExtra;
  assert.equal((typeof unknown === 'function' ? unknown() : unknown)?.find(row => row.label === 'Killer country')?.value.kind, 'notRecorded');
});

test('the roster is refused without the portrait pair', () => {
  const { portrait: _portrait, ...old } = ace('1', 'GER');
  assert.throws(() => validateAces(reply([old as AirAce])));
  assert.throws(() => validateAces(reply([ace('1', 'GER', { portrait: null, portraitMissing: null })])));
});
