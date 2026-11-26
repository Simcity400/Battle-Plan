import assert from 'node:assert/strict';
import test from 'node:test';
import { factionNameChanges, factionNamer, validateAirSuperiority } from './airSuperiority.ts';
import type { AirSide, AirSuperiorityReply, FactionGroup } from '../generated/dashboard';

const side = (key: string, faction: string | null, leader: string | null): AirSide =>
  ({ key, faction, colourFaction: null, factions: faction ? [faction] : [], tag: faction ? null : leader, leader, coBelligerents: 0, anchor: null });
const SIDES = [side('f:7', '7', 'ENG'), side('c:GER', null, 'GER'), side('c:USA', null, 'USA')];
const reply: AirSuperiorityReply = {
  missing: null, unlisted: null, empty: null, activeUnlisted: { ids: [], missing: null }, sides: SIDES, legend: [0, 1], viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
  fallback: { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 },
  regions: [
    { id: 10, name: 'North Sea', nameMissing: null, active: true, pinned: false, wings: 'live',
      cell: { kind: 'sides', parts: [{ side: 1, tag: 'GER', live: 'GER@10' }, { side: 0, tag: 'ENG', live: 'ENG@10' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 } },
    { id: 11, name: 'Baltic', nameMissing: null, active: true, pinned: false, wings: 'none',
      cell: { kind: 'sides', parts: [{ side: 0, tag: 'ENG', live: 'ENG@11' }, { side: 1, tag: 'GER', live: 'GER@11' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 } },
    { id: 12, name: 'Atlantic', nameMissing: null, active: false, pinned: false, wings: 'unknown', cell: { kind: 'reason', reason: 'outside-theatres' } },
  ],
};

test('a reply naming a side it does not hold is refused', () => {
  assert.equal(validateAirSuperiority(reply), reply);
  assert.throws(() => validateAirSuperiority({ ...reply, regions: [{ ...reply.regions[0], cell: { ...reply.regions[0].cell, parts: [{ side: 9, tag: 'X', live: 'X@1' }] } }] }));
  assert.throws(() => validateAirSuperiority({ ...reply, scale: 0 }));
});

test('a side chooser names each faction as its branch named it at the widget\'s hour, and says when no name is recorded then', () => {
  const run = (firstHour: number, lastHour: number, name: string | null) => ({ firstHour, lastHour, name, reason: name ? null : 'Not recorded', token: null });
  const group = (names: FactionGroup['names'], name: string): FactionGroup => ({ id: '7', name, nameReason: null, names,
    recordedName: name, firstHour: 10, lastHour: 99, members: [], leaders: [], incomplete: [] });
  const branch = group([run(10, 20, 'Old League'), run(21, 46, null), run(47, 99, 'New League')], 'New League');
  const campaign = group([run(10, 99, 'Other Branch League')], 'Other Branch League');
  const named = (at: number | 'not-recorded' | null, own = new Map<string, string>()) => factionNamer([branch], [campaign], own, at === 'not-recorded' ? null : at, at === 'not-recorded')('7');
  assert.equal(named(15), 'Old League', 'the name the widget\'s own branch recorded at the hour');
  assert.equal(named(30), 'Name not recorded', 'an hour with no recorded name never shows a later rename');
  assert.equal(named(60), 'New League');
  assert.equal(named('not-recorded'), 'Name not recorded', 'a gap in the recording is not the campaign\'s latest name');
  assert.equal(named(null), 'Other Branch League', 'outside the recording the chooser keeps the campaign-wide name');
  assert.equal(factionNamer([], [campaign], new Map(), 60, false)('7'), 'Name not recorded', 'a faction the branch does not hold is not named from elsewhere');
  assert.equal(factionNamer(undefined, [campaign], new Map(), 60, false)('7'), 'Name not recorded', 'while the branch groups load, a displayed hour is not given the campaign-wide name');
  assert.equal(factionNamer(undefined, [campaign], new Map(), null, true)('7'), 'Name not recorded', 'a gap stays a gap while the branch groups load');
  assert.equal(named(15, new Map([['7', 'My allies']])), 'My allies', "the user's own name for a faction wins");
  assert.deepEqual(factionNameChanges([branch]), [10, 21, 47, 100]);
});
