import { test } from 'node:test';
import assert from 'node:assert/strict';
import { validatePlacements } from '../../dashboard/intelligence.ts';
import { OTHER_ID, countryNodeId, operativeNames, placementChord, placementRows, placementStructure, placementTooltip } from './placementsData.ts';
import type { Placement } from '../../generated/dashboard';

const operative = (id: number, name: string | null) => ({ id: String(id), name, nameMissing: name === null ? 'The operative has no saved name' : null });
const placement = (owner: string, target: string, count: number): Placement => ({
  owner: { id: owner.charCodeAt(0) * 1000 + owner.charCodeAt(1), tag: owner }, target: { id: target.charCodeAt(0) * 1000 + target.charCodeAt(1), tag: target },
  operatives: Array.from({ length: count }, (_, at) => operative(at + 1, at === 0 ? null : `Agent ${at}`)),
});
const look = (country: { id: number; tag: string | null }) => ({ name: country.tag ?? `Country ${country.id}` });

test('the chord draws the countries sending and receiving the most operatives and folds the rest into one', () => {
  const placements = [placement('GER', 'ENG', 3), placement('ENG', 'GER', 1), placement('ITA', 'FRA', 1), placement('SOV', 'GER', 2)];
  const chord = placementChord(placementRows(placements, null), 3, look, () => '#000', '#999');
  const id = (at: number, side: 'owner' | 'target') => countryNodeId(placements[at][side]);
  assert.deepEqual([...chord.nodes.keys()].sort(), [id(0, 'owner'), id(0, 'target'), id(3, 'owner'), OTHER_ID].sort());
  assert.equal(chord.nodes.get(OTHER_ID)!.length, 2);
  const tip = placementTooltip({ kind: 'node', id: id(0, 'owner') }, chord)!;
  assert.deepEqual(tip.rows!.map(row => row.value), ['3', '3']);
});

test('the structure names the countries and why nothing is known, never the counts', () => {
  const reply = (count: number, target = 'ENG') => ({ placements: [placement('GER', target, count)], missing: null });
  assert.equal(placementStructure(reply(1)), placementStructure(reply(3)));
  assert.notEqual(placementStructure(reply(1)), placementStructure(reply(1, 'FRA')));
});

test('chosen countries keep every pair touching them', () => {
  const placements = [placement('GER', 'ENG', 3), placement('ITA', 'FRA', 1), placement('SOV', 'ITA', 2)];
  assert.deepEqual(placementRows(placements, new Set(['ITA'])).map(row => row.owner.tag), ['SOV', 'ITA']);
});

test('an operative without a name says so, and a pair without operatives is refused', () => {
  assert.equal(operativeNames(placement('GER', 'ENG', 2)), 'Unnamed operative, Agent 1');
  assert.throws(() => validatePlacements({ placements: [placement('GER', 'ENG', 0)], readHour: null, missing: null, warnings: [] }));
});
