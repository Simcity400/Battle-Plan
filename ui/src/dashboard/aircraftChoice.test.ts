import assert from 'node:assert/strict';
import test from 'node:test';
import { aircraftTitle, loadAircraftChoice, saveAircraftChoice } from './aircraftChoice.ts';
import { recordingKey } from './campaigns.ts';

function memory() {
  const items = new Map<string, string>();
  return { items, getItem: (key: string) => items.get(key) ?? null, setItem: (key: string, value: string) => { items.set(key, value); },
    removeItem: (key: string) => { items.delete(key); } };
}

const FIGHTER = { name: 'GER_fighter_equipment_1_entity', title: 'Fighter I' };
const JET = { name: 'jet_mesh', title: 'Jet' };

test('each recording keeps its own choice, with the title it is shown by, and none is the default', () => {
  const storage = memory();
  assert.equal(loadAircraftChoice('C:/a.battleplan', storage), null);
  saveAircraftChoice('C:/a.battleplan', FIGHTER, storage);
  saveAircraftChoice('C:/b.battleplan', JET, storage);
  assert.deepEqual(loadAircraftChoice('C:/a.battleplan', storage), FIGHTER);
  assert.deepEqual(loadAircraftChoice('C:/b.battleplan', storage), JET);
  saveAircraftChoice('C:/a.battleplan', null, storage);
  assert.equal(loadAircraftChoice('C:/a.battleplan', storage), null);
  assert.equal(storage.items.size, 1);
  storage.items.set('battleplan:fly-aircraft:C:/c.battleplan', 'ITA_fighter_equipment_1_entity');
  assert.deepEqual(loadAircraftChoice('C:/c.battleplan', storage), { name: 'ITA_fighter_equipment_1_entity', title: 'ITA fighter equipment 1' },
    'a choice saved before titles were kept flies the same model without showing its registered name');
  storage.items.set('battleplan:fly-aircraft:C:/d.battleplan', '{"title":"Fighter I"}');
  assert.equal(loadAircraftChoice('C:/d.battleplan', storage), null);
  assert.equal(aircraftTitle({ titles: ['Fighter I', 'Fighter II'], name: 'fighter_entity' }), 'Fighter I', 'the first title names it, as the list sorts it');
  assert.equal(aircraftTitle({ titles: [], name: 'jet_mesh' }), 'Jet');
  assert.equal(aircraftTitle({ titles: [], name: 'GER_cas_equipment_2_entity' }), 'GER cas equipment 2');
});

test('storage that cannot be used keeps no choice and throws nothing', () => {
  const broken = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('full'); }, removeItem: () => { throw new Error('denied'); } };
  assert.equal(loadAircraftChoice('C:/a.battleplan', broken), null);
  assert.doesNotThrow(() => saveAircraftChoice('C:/a.battleplan', JET, broken));
  assert.doesNotThrow(() => saveAircraftChoice('C:/a.battleplan', null, broken));
});

const campaignKey = (id: string) => [...id].map(character => character.charCodeAt(0).toString(16).padStart(2, '0')).join('');
const GERMANY = campaignKey('00000000-0000-4000-8000-000000000001');
const ITALY = campaignKey('00000000-0000-4000-8000-000000000002');

test('a recording keeps its choice by folder and campaign, so a rename keeps it', () => {
  const storage = memory();
  saveAircraftChoice(recordingKey(String.raw`\\?\C:\Recordings\Germany 1936.battleplan`, GERMANY), JET, storage);
  assert.deepEqual(loadAircraftChoice(recordingKey('c:/recordings/Germany 1936–1937.battleplan', GERMANY), storage), JET);
  assert.equal(loadAircraftChoice(recordingKey('C:/Recordings/Germany (2)/Germany 1936.battleplan', GERMANY), storage), null);
  assert.equal(loadAircraftChoice(recordingKey('C:/Recordings/Germany 1936.battleplan', ITALY), storage), null);
});

test('a recording whose header names no campaign keeps its choice by path', () => {
  const storage = memory();
  saveAircraftChoice(recordingKey('C:/Recordings/a.battleplan', 'ab'), JET, storage);
  assert.deepEqual(loadAircraftChoice(recordingKey('C:/Recordings/a.battleplan', 'ab'), storage), JET);
  assert.equal(loadAircraftChoice(recordingKey('C:/Recordings/b.battleplan', 'ab'), storage), null);
});
