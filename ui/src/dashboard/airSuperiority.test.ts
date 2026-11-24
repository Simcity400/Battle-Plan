import assert from 'node:assert/strict';
import test from 'node:test';
import { validateAirSuperiority } from './airSuperiority.ts';
import type { AirSide, AirSuperiorityReply } from '../generated/dashboard';

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
