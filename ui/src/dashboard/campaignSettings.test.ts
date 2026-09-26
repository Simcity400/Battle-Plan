import test from 'node:test';
import assert from 'node:assert/strict';
import { applyColourSet, loadCampaignSettings, MAX_FACTIONS, parseCampaignSettings, saveCampaignSettings, saveColourSet, setFactionStyle } from './campaignSettings.ts';
import { factionColours } from '../charts/factionColours.ts';
import { contrast, parseColor, rgbToOklch } from '../shell/appearance/color.ts';
import { distance, GREY_CHROMA } from '../charts/palette.ts';

const memory = () => { const values = new Map<string, string>(); return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } }; };
const FACTIONS = [{ id: '12', name: 'Axis' }, { id: '3', name: 'Allies' }, { id: '40', name: null }];

test('faction styles are stored per campaign, validated and cleared', () => {
  let settings = parseCampaignSettings(undefined);
  settings = setFactionStyle(settings, 'ab12', '3', { colour: '#1A2B3C', name: '  Western Allies ' });
  assert.deepEqual(settings.campaigns.ab12.factions['3'], { colour: '#1a2b3c', name: 'Western Allies' });
  assert.equal(setFactionStyle(settings, 'ab12', '3', { colour: 'purple-ish' }), settings);
  settings = setFactionStyle(settings, 'ab12', '3', null);
  assert.deepEqual(settings.campaigns.ab12.factions, {});
  const storage = memory();
  saveCampaignSettings(setFactionStyle(settings, 'ab12', '12', { colour: '#aa0000' }), storage);
  assert.deepEqual(loadCampaignSettings(storage).campaigns.ab12.factions['12'], { colour: '#aa0000' });
});

test('unsafe or malformed stored settings are dropped', () => {
  const parsed = parseCampaignSettings({ version: 1, campaigns: {
    ab: { factions: { 1: { colour: '#00ff00' }, x: { colour: '#00ff00' }, 2: { colour: 'nope' } } }, 'not-hex': { factions: {} }, __proto__: { factions: {} } },
    sets: [{ id: 'mp', name: 'MP group', entries: [{ faction: 'Axis', colour: '#ff0000' }, { faction: '', colour: '#ff0000' }] }, { id: 'bad id!', name: 'x', entries: [] }] });
  assert.deepEqual(Object.keys(parsed.campaigns), ['ab']);
  assert.deepEqual(parsed.campaigns.ab.factions, { 1: { colour: '#00ff00' } });
  assert.deepEqual(parsed.sets, [{ id: 'mp', name: 'MP group', entries: [{ faction: 'Axis', colour: '#ff0000' }] }]);
  assert.deepEqual(parseCampaignSettings({ version: 2 }), { version: 1, campaigns: {}, sets: [] });
});

test('colour sets save by faction name and apply to another campaign, ignoring case', () => {
  let settings = setFactionStyle(parseCampaignSettings(undefined), 'aa', '12', { colour: '#222222', name: 'The Axis' });
  settings = saveColourSet(settings, 'mp', 'MP group', 'aa', FACTIONS, new Map([['3', '#3366cc'], ['12', '#999999']]));
  assert.deepEqual(settings.sets[0].entries, [{ faction: 'Axis', colour: '#222222', name: 'The Axis' }, { faction: 'Allies', colour: '#3366cc' }]);
  const { settings: applied, matched } = applyColourSet(settings, settings.sets[0], 'bb', [{ id: '7', name: 'AXIS' }, { id: '8', name: 'Comintern' }]);
  assert.equal(matched, 1);
  assert.deepEqual(applied.campaigns.bb.factions, { 7: { colour: '#222222', name: 'The Axis' } });
});

test('default faction colours: leader colour or palette, readable, never grey, apart and stable', () => {
  const surface = '#16181d';
  const background = parseColor(surface)!;
  const factions = [{ id: '12', leaderColor: parseColor('#808080') }, { id: '3', leaderColor: null }, { id: '40', leaderColor: parseColor('#3a6ea5') }];
  const colours = factionColours(factions, {}, surface);
  assert.deepEqual(factionColours([...factions].reverse(), {}, surface), colours);
  const values = [...colours.values()].map(hex => parseColor(hex)!);
  for (const value of values) {
    assert.ok(contrast(value, background) >= 2.95);
    assert.ok(rgbToOklch(value).c >= GREY_CHROMA * 0.9);
  }
  assert.ok(distance(values[0], values[1]) >= 12 && distance(values[1], values[2]) >= 12 && distance(values[0], values[2]) >= 12);
  const chosen = factionColours(factions, { 40: '#3a6ea5' }, surface);
  assert.equal(chosen.get('40'), '#3a6ea5');
});

test('defaults keep away from chosen colours as they are shown, even pale ones', () => {
  const surface = '#16181d';
  const colours = factionColours(['1', '2', '3', '4'].map(id => ({ id })), { 1: '#d8ebf7', 2: '#79c2ed' }, surface);
  assert.equal(colours.get('1'), '#d8ebf7');
  for (const id of ['3', '4']) {
    for (const chosen of ['1', '2']) assert.ok(distance(parseColor(colours.get(id)!)!, parseColor(colours.get(chosen)!)!) >= 12, `${id} vs ${chosen}`);
  }
});

test('a new faction style past the limit is refused, not dropped on save', () => {
  let settings = parseCampaignSettings(undefined);
  for (let id = 0; id < MAX_FACTIONS; id++) settings = setFactionStyle(settings, 'aa', String(id), { colour: '#112233' });
  assert.equal(setFactionStyle(settings, 'aa', '9999', { colour: '#112233' }), settings);
  assert.notEqual(setFactionStyle(settings, 'aa', '5', { colour: '#445566' }), settings);
});
