import assert from 'node:assert/strict';
import test from 'node:test';
import { loadAircraftChoice, saveAircraftChoice } from './aircraftChoice.ts';

function memory() {
  const items = new Map<string, string>();
  return { items, getItem: (key: string) => items.get(key) ?? null, setItem: (key: string, value: string) => { items.set(key, value); },
    removeItem: (key: string) => { items.delete(key); } };
}

test('each recording keeps its own choice and none is the default', () => {
  const storage = memory();
  assert.equal(loadAircraftChoice('C:/a.battleplan', storage), null);
  saveAircraftChoice('C:/a.battleplan', 'GER_fighter_equipment_1_entity', storage);
  saveAircraftChoice('C:/b.battleplan', 'jet_mesh', storage);
  assert.equal(loadAircraftChoice('C:/a.battleplan', storage), 'GER_fighter_equipment_1_entity');
  assert.equal(loadAircraftChoice('C:/b.battleplan', storage), 'jet_mesh');
  saveAircraftChoice('C:/a.battleplan', null, storage);
  assert.equal(loadAircraftChoice('C:/a.battleplan', storage), null);
  assert.equal(storage.items.size, 1);
});

test('storage that cannot be used keeps no choice and throws nothing', () => {
  const broken = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('full'); }, removeItem: () => { throw new Error('denied'); } };
  assert.equal(loadAircraftChoice('C:/a.battleplan', broken), null);
  assert.doesNotThrow(() => saveAircraftChoice('C:/a.battleplan', 'plane', broken));
  assert.doesNotThrow(() => saveAircraftChoice('C:/a.battleplan', null, broken));
});
