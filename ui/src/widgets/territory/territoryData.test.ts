import assert from 'node:assert/strict';
import test from 'node:test';
import { entryCell, shareItems, shareStructure, validateShareReply, WORLD_TOTAL } from './territoryData.ts';
import type { TerritoryShareReply } from '../../generated/dashboard';


const values = (cells: Record<string, number>) => ({ get: (key: string) => cells[key], missing: (key: string) => key in cells ? '' : 'Not read yet' });

test('a share is the first countries at their cells and the rest of every country, and none where the whole is not known', () => {
  const reply: TerritoryShareReply = { measure: 'controlledStates', entries: [{ tag: 'GER', slot: 1 }, { tag: 'SOV', slot: 2 }, { tag: 'ITA', slot: 3 }],
    unnamed: true, missing: null, warnings: [] };
  assert.equal(validateShareReply(reply), reply);
  const look = (tag: string) => ({ id: tag, label: tag });
  const cells = values({ [WORLD_TOTAL]: 100, [entryCell(1)]: 50, [entryCell(2)]: 30, [entryCell(3)]: 15 });
  const { items, unavailableReason } = shareItems(reply, 2, cells, look, { id: 'rest', label: 'Rest' });
  assert.equal(unavailableReason, null);
  assert.deepEqual(items.map(item => [item.path[0].id, item.value, item.exact]), [['GER', 50, 50n], ['SOV', 30, 30n], ['rest', 20, 20n]]);
  assert.equal(items[2].path[0].last, true, 'the rest comes after the named slices');
  const missing = '1 countries are not recorded at this hour';
  assert.deepEqual(shareItems({ ...reply, missing }, 2, cells, look, { id: 'rest', label: 'Rest' }), { items: [], unavailableReason: missing });
  assert.equal(shareItems(reply, 2, values({ [entryCell(1)]: 50 }), look, { id: 'rest', label: 'Rest' }).unavailableReason, 'Not read yet', 'the world not read: no whole');
  const partial = shareItems(reply, 2, values({ [WORLD_TOTAL]: 100, [entryCell(1)]: 50 }), look, { id: 'rest', label: 'Rest' });
  assert.deepEqual(partial.items.map(item => [item.path[0].id, item.value]), [['GER', 50], ['SOV', null]], 'a slice not known is not measured, and no rest is guessed');
  assert.throws(() => validateShareReply({ ...reply, unnamed: 5 }));
});

test('a share renders for the countries it draws: those below them trading places, as the service orders them by amount, change nothing', () => {
  const entry = (tag: string, slot: number) => ({ tag, slot });
  const reply = (...entries: [string, number][]): TerritoryShareReply => ({ measure: 'controlledStates', entries: entries.map(([tag, slot]) => entry(tag, slot)),
    unnamed: false, missing: null, warnings: [] });
  const before = shareStructure(reply(['GER', 1], ['SOV', 2], ['ITA', 3], ['JAP', 4]), 2);
  assert.equal(shareStructure(reply(['GER', 1], ['SOV', 2], ['JAP', 4], ['ITA', 3]), 2), before, 'the countries in the rest traded places');
  assert.equal(shareStructure(reply(['GER', 1], ['SOV', 2], ['ITA', 3]), 2), before, 'one left the rest');
  assert.notEqual(shareStructure(reply(['SOV', 2], ['GER', 1], ['ITA', 3], ['JAP', 4]), 2), before, 'the slices drawn traded places');
  assert.notEqual(shareStructure(reply(['GER', 1], ['ITA', 3], ['SOV', 2], ['JAP', 4]), 2), before, 'another country is drawn');
  assert.notEqual(shareStructure({ ...reply(['GER', 1], ['SOV', 2], ['ITA', 3], ['JAP', 4]), missing: 'not recorded' }, 2), before, 'the whole is missing');
});
