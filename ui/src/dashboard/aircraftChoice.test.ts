import assert from 'node:assert/strict';
import test from 'node:test';
import { loadAircraftChoice, saveAircraftChoice } from './aircraftChoice.ts';
import { recordingKey } from './campaigns.ts';

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

/** A header's campaign id as its campaign key: the id's bytes in hex. */
const campaignKey = (id: string) => [...id].map(character => character.charCodeAt(0).toString(16).padStart(2, '0')).join('');
const GERMANY = campaignKey('00000000-0000-4000-8000-000000000001');
const ITALY = campaignKey('00000000-0000-4000-8000-000000000002');

test('a recording keeps its choice by folder and campaign, so a rename keeps it', () => {
  const storage = memory();
  saveAircraftChoice(recordingKey(String.raw`\\?\C:\Recordings\Germany 1936.battleplan`, GERMANY), 'jet_mesh', storage);
  assert.equal(loadAircraftChoice(recordingKey('c:/recordings/Germany 1936–1937.battleplan', GERMANY), storage), 'jet_mesh');
  assert.equal(loadAircraftChoice(recordingKey('C:/Recordings/Germany (2)/Germany 1936.battleplan', GERMANY), storage), null);
  assert.equal(loadAircraftChoice(recordingKey('C:/Recordings/Germany 1936.battleplan', ITALY), storage), null);
});

test('a recording whose header names no campaign keeps its choice by path', () => {
  const storage = memory();
  saveAircraftChoice(recordingKey('C:/Recordings/a.battleplan', 'ab'), 'jet_mesh', storage);
  assert.equal(loadAircraftChoice(recordingKey('C:/Recordings/a.battleplan', 'ab'), storage), 'jet_mesh');
  assert.equal(loadAircraftChoice(recordingKey('C:/Recordings/b.battleplan', 'ab'), storage), null);
});
