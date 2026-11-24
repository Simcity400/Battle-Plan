import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../charts/live.ts';
import { airPercent, airState, airTooltip, cellView, sideOrder, validateAirSuperiority } from './airSuperiority.ts';
import type { AirLooks } from './airSuperiority';
import type { AirSide, AirSuperiorityReply } from '../generated/dashboard';

const side = (key: string, faction: string | null, leader: string | null): AirSide =>
  ({ key, faction, colourFaction: null, factions: faction ? [faction] : [], tag: faction ? null : leader, leader, coBelligerents: 0, anchor: null });
const SIDES = [side('f:7', '7', 'ENG'), side('c:GER', null, 'GER'), side('c:USA', null, 'USA')];
const looks: AirLooks = { name: s => s.leader ?? s.key, color: s => `#${s.key.length}00`, viewpoint: { name: 'England', tag: 'ENG', color: '#111' }, enemy: '#f00',
  scale: 100_000, badBelow: 40_000, goodAbove: 60_000, mode: 'sides' };
const values = (entries: Record<string, number>) => valuesOf(new Map(Object.entries(entries).map(([key, value]) => {
  const [row, column] = key.split('.'); return [liveKey(row, column), value] as const;
})), 'No live row has this key');

const reply: AirSuperiorityReply = {
  missing: null, sides: SIDES, legend: [0, 1], viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
  fallback: { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 },
  regions: [
    { id: 10, name: 'North Sea', nameMissing: null, active: true, pinned: false, wings: 'live',
      cell: { kind: 'sides', parts: [{ side: 1, tag: 'GER', live: 'GER@10' }, { side: 0, tag: 'ENG', live: 'ENG@10' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 } },
    { id: 11, name: 'Baltic', nameMissing: null, active: true, pinned: false, wings: 'none',
      cell: { kind: 'sides', parts: [{ side: 0, tag: 'ENG', live: 'ENG@11' }, { side: 1, tag: 'GER', live: 'GER@11' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 } },
    { id: 12, name: 'Atlantic', nameMissing: null, active: false, pinned: false, wings: 'unknown', cell: { kind: 'reason', reason: 'outside-theatres' } },
  ],
};

test('a region\'s shares are its sides\' friendly scores read from the live cells, a zero part left out and factions first', () => {
  const order = sideOrder(SIDES);
  const view = cellView(reply.regions[0].cell, SIDES, values({ 'GER@10.friendly': 300, 'ENG@10.friendly': 100 }), looks, order, 3);
  assert.equal(view.state, 'known');
  assert.deepEqual(view.parts.map(part => [part.id, part.score, part.share]), [['f:7', 100, 0.25], ['c:GER', 300, 0.75]]);
  assert.equal(airPercent(100, view.total), '25%');
  const zero = cellView(reply.regions[0].cell, SIDES, values({ 'GER@10.friendly': 0, 'ENG@10.friendly': 100 }), looks, order, 3);
  assert.deepEqual(zero.parts.map(part => part.id), ['f:7']);
  const missing = cellView(reply.regions[0].cell, SIDES, values({ 'ENG@10.friendly': 100 }), looks, order, 3);
  assert.equal(missing.state, 'unknown', 'a part without a followed value is unknown, never zero');
  assert.equal(missing.reason, 'No live row has this key');
});

test('a viewpoint\'s state is the game\'s strict thresholds over friendly against enemy', () => {
  const cell = { kind: 'view' as const, side: 0, leader: 'ENG', live: 'ENG@10' };
  const viewpoint = { ...looks, mode: 'viewpoint' as const };
  const at = (friendly: number, enemy: number) => airState(cellView(cell, SIDES, values({ 'ENG@10.friendly': friendly, 'ENG@10.enemy': enemy }), viewpoint, sideOrder(SIDES), null));
  assert.equal(at(61, 39), 'Air superiority');
  assert.equal(at(60, 40), 'Contested');
  assert.equal(at(39, 61), 'Enemy superiority');
  assert.equal(at(0, 0), 'No air activity');
  const zero = cellView({ ...cell, live: null }, SIDES, values({}), viewpoint, sideOrder(SIDES), null);
  assert.equal(zero.state, 'inactive', 'a known zero has no row and reads as no activity');
  const tooltip = airTooltip({ name: 'North Sea' }, cellView(cell, SIDES, values({ 'ENG@10.friendly': 75, 'ENG@10.enemy': 25 }), viewpoint, sideOrder(SIDES), 2), true,
    tag => ({ name: `Country ${tag}` }), false, id => `Faction ${id}`);
  assert.ok(tooltip.rows.some(row => row.label === 'State' && row.value === 'Air superiority'));
  assert.ok(tooltip.rows.some(row => row.label === 'Air wings assigned to the region' && row.value === '2'));
});

test('a reply naming a side it does not hold is refused', () => {
  assert.equal(validateAirSuperiority(reply), reply);
  assert.throws(() => validateAirSuperiority({ ...reply, regions: [{ ...reply.regions[0], cell: { ...reply.regions[0].cell, parts: [{ side: 9, tag: 'X', live: 'X@1' }] } }] }));
  assert.throws(() => validateAirSuperiority({ ...reply, scale: 0 }));
});
