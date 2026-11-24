import test from 'node:test';
import assert from 'node:assert/strict';
import { chosenColour, counterCountries, factionIdentity, factionStylesKey, loadCampaignSettings, MAX_CAMPAIGN_SETTINGS_STORAGE, parseCampaignSettings, saveCampaignSettings,
  setCampaignFlag, setCounterCountries, setFactionColour, setFactionStyle } from './campaignSettings.ts';
import { FACTION_APART, factionColours } from '../charts/factionColours.ts';
import { parseColor, rgbToOklch } from '../shell/appearance/color.ts';
import { categoricalPalette, distance } from '../charts/palette.ts';

const memory = () => { const values = new Map<string, string>(); return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } }; };

test('campaign flags preserve opaque recorded tags and faction settings through save, edit and clear', () => {
  const legacy = parseCampaignSettings({ version: 1, campaigns: { ab: { factions: { 3: { colour: '#123456', name: 'Friends' } } } }, sets: [] });
  assert.equal(legacy.campaigns.ab.flag, undefined);
  let settings = setCampaignFlag(legacy, 'ab', 'mod-country-long-tag');
  const storage = memory();
  assert.equal(saveCampaignSettings(settings, storage), null);
  settings = loadCampaignSettings(storage);
  assert.equal(settings.campaigns.ab.flag, 'mod-country-long-tag');
  assert.deepEqual(settings.campaigns.ab.factions, legacy.campaigns.ab.factions);
  settings = setFactionStyle(settings, 'ab', '4', { colour: '#abcdef' });
  assert.equal(settings.campaigns.ab.flag, 'mod-country-long-tag', 'writing faction settings must retain the flag');
  settings = setCampaignFlag(settings, 'ab', null);
  assert.equal(settings.campaigns.ab.flag, undefined);
  assert.deepEqual(settings.campaigns.ab.factions, { 3: { colour: '#123456', name: 'Friends' }, 4: { colour: '#abcdef' } });
});

test('chosen counter countries keep per campaign and map scope through save and reload, alongside flags and factions', () => {
  let settings = setFactionStyle(setCampaignFlag(parseCampaignSettings(undefined), 'ab', 'GER'), 'ab', '3', { colour: '#123456' });
  settings = setCounterCountries(settings, 'ab', 'campaign', ['GER', 'ITA', 'GER']);
  settings = setCounterCountries(settings, 'ab', 'widget:map-1', []);
  settings = setCounterCountries(settings, 'cd', 'campaign', ['mod country tag']);
  const storage = memory();
  assert.equal(saveCampaignSettings(settings, storage), null);
  const loaded = loadCampaignSettings(storage);
  assert.deepEqual(counterCountries(loaded, 'ab', 'campaign'), ['GER', 'ITA']);
  assert.deepEqual(counterCountries(loaded, 'ab', 'widget:map-1'), [], 'an emptied list is a choice, not a missing one');
  assert.equal(counterCountries(loaded, 'ab', 'widget:map-2'), undefined);
  assert.deepEqual(counterCountries(loaded, 'cd', 'campaign'), ['mod country tag']);
  assert.equal(loaded.campaigns.ab.flag, 'GER');
  assert.deepEqual(loaded.campaigns.ab.factions, { 3: { colour: '#123456' } });
  const tags = Array.from({ length: 1500 }, (_, index) => `T${index}`);
  assert.equal(saveCampaignSettings(setCounterCountries(loaded, 'ab', 'campaign', tags), storage), null);
  assert.deepEqual(counterCountries(loadCampaignSettings(storage), 'ab', 'campaign'), tags);
  const parsed = parseCampaignSettings({ version: 1, campaigns: { ab: { factions: {}, counterCountries: { campaign: ['GER', '', 3, 'a\0b'], __proto__: ['X'], 'widget:x': 'GER' } } } });
  assert.deepEqual(parsed.campaigns.ab.counterCountries, { campaign: ['GER'] });
});

test('campaign flag validation drops malformed flags while preserving factions', () => {
  for (const flag of ['', '\0', 'mod\0country', null, undefined, 3, {}, []]) {
    const settings = parseCampaignSettings({ version: 1, campaigns: { ab: { flag, factions: { 3: { colour: '#123456' } } } } });
    assert.deepEqual(settings.campaigns.ab, { factions: { 3: { colour: '#123456' } } });
    if (flag !== null) assert.equal(setCampaignFlag(settings, 'ab', flag as string), settings);
  }
});

test('campaign flags accept complete long tags and whitespace exactly as recorded', () => {
  for (const flag of ['x'.repeat(5000), '   ', ' mod-country ']) {
    const settings = setCampaignFlag(parseCampaignSettings(undefined), 'ab', flag);
    const storage = memory();
    assert.equal(saveCampaignSettings(settings, storage), null);
    assert.equal(loadCampaignSettings(storage).campaigns.ab.flag, flag);
  }
});

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

test('the faction styles a campaign renders with change with its own names and colours and every campaign’s colours, not another campaign’s', () => {
  const empty = parseCampaignSettings(undefined);
  assert.equal(factionStylesKey(empty, 'ab12'), '');
  assert.equal(factionStylesKey(empty, null), '');
  const renamed = setFactionStyle(empty, 'ab12', '3', { name: 'Western Allies' });
  assert.notEqual(factionStylesKey(renamed, 'ab12'), factionStylesKey(empty, 'ab12'));
  assert.equal(factionStylesKey(setFactionStyle(empty, 'other', '3', { name: 'Western Allies' }), 'ab12'), '');
  const shared = setFactionColour(renamed, 'all', 'ab12', '3', factionIdentity('Allies', null), '#123456');
  assert.notEqual(factionStylesKey(shared, 'ab12'), factionStylesKey(renamed, 'ab12'));
  assert.equal(factionStylesKey(setFactionStyle(renamed, 'ab12', '3', { name: 'Western Allies' }), 'ab12'), factionStylesKey(renamed, 'ab12'));
});

test('unsafe or malformed stored settings are dropped', () => {
  const parsed = parseCampaignSettings({ version: 1, campaigns: {
    ab: { factions: { 1: { colour: '#00ff00' }, x: { colour: '#00ff00' }, 2: { colour: 'nope' } } }, 'not-hex': { factions: {} }, __proto__: { factions: {} } },
    sets: [{ id: 'mp', name: 'MP group', entries: [{ faction: 'Axis', colour: '#ff0000' }, { faction: '', colour: '#ff0000' }] }, { id: 'bad id!', name: 'x', entries: [] }] });
  assert.deepEqual(Object.keys(parsed.campaigns), ['ab']);
  assert.deepEqual(parsed.campaigns.ab.factions, { 1: { colour: '#00ff00' } });
  assert.deepEqual(parsed.sets, [{ id: 'mp', name: 'MP group', entries: [{ faction: 'Axis', colour: '#ff0000' }] }]);
  assert.deepEqual(parseCampaignSettings({ version: 2 }), { version: 1, campaigns: {}, shared: {}, sets: [] });
  assert.deepEqual(parseCampaignSettings({ version: 1, campaigns: {}, sets: [] }).shared, {});
  assert.deepEqual(parseCampaignSettings({ version: 1, campaigns: {}, sets: [], shared: { '["name","axis"]': { colour: '#AA0000' }, x: { colour: 'nope' }, __proto__: { colour: '#000000' } } }).shared,
    { '["name","axis"]': { colour: '#aa0000' } });
});

test('default faction colours are the leader colour exactly, grey included, else the palette, and stable', () => {
  const surface = '#16181d';
  const factions = [{ id: '12', leaderColor: parseColor('#808080') }, { id: '3', leaderColor: null }, { id: '40', leaderColor: parseColor('#3a6ea5') }];
  const colours = factionColours(factions, {}, surface);
  assert.deepEqual(factionColours([...factions].reverse(), {}, surface), colours);
  assert.equal(colours.get('12'), '#808080');
  assert.equal(colours.get('40'), '#3a6ea5');
  assert.equal(colours.get('3'), categoricalPalette(surface)[0]);
  assert.equal(factionColours(factions, { 40: '#123456' }, surface).get('40'), '#123456');
});

test('only clashing default colours are nudged, slightly, by lightness, a grey staying grey', () => {
  const surface = '#16181d';
  const colours = factionColours([
    { id: '1', leaderColor: parseColor('#808080') }, { id: '2', leaderColor: parseColor('#808080') },
    { id: '3', leaderColor: parseColor('#3a6ea5') }, { id: '4', leaderColor: parseColor('#3b6fa6') },
  ], {}, surface);
  assert.equal(colours.get('1'), '#808080');
  assert.equal(colours.get('3'), '#3a6ea5');
  const [first, second, blue, nearBlue] = ['1', '2', '3', '4'].map(id => parseColor(colours.get(id)!)!);
  assert.ok(distance(first, second) >= FACTION_APART && distance(first, second) < FACTION_APART + 2);
  assert.ok(distance(blue, nearBlue) >= FACTION_APART && distance(blue, nearBlue) < FACTION_APART + 2);
  assert.ok(rgbToOklch(second).c < 0.01);
  assert.ok(Math.abs(rgbToOklch(nearBlue).h - rgbToOklch(blue).h) < 3);
  const around = factionColours([{ id: '1', leaderColor: parseColor('#808080') }, { id: '2' }], { 2: '#808080' }, surface);
  assert.equal(around.get('2'), '#808080');
  assert.ok(distance(parseColor(around.get('1')!)!, parseColor('#808080')!) >= FACTION_APART);
});

test('a colour set for all campaigns applies wherever the faction is found; a campaign can override it', () => {
  const axis = factionIdentity('Axis', 'GER');
  assert.equal(axis, factionIdentity('Axis', 'ITA'));
  assert.notEqual(factionIdentity('FACTION_PACT', null), factionIdentity('The Pact', null));
  assert.equal(factionIdentity(null, 'D01'), '["leader","D01"]');
  assert.equal(factionIdentity(null, null), null);
  let settings = parseCampaignSettings(undefined);
  settings = setFactionStyle(settings, 'aa', '12', { colour: '#111111', name: 'The Axis' });
  settings = setFactionColour(settings, 'all', 'aa', '12', axis, '#AA0000');
  assert.deepEqual(settings.shared, { [axis!]: { colour: '#aa0000' } });
  assert.deepEqual(settings.campaigns.aa.factions['12'], { name: 'The Axis' });
  assert.equal(chosenColour(settings, 'aa', '12', axis), '#aa0000');
  assert.equal(chosenColour(settings, 'bb', '99', axis), '#aa0000');
  settings = setFactionColour(settings, 'campaign', 'bb', '99', axis, '#00aa00');
  assert.equal(chosenColour(settings, 'bb', '99', axis), '#00aa00');
  assert.equal(chosenColour(settings, 'aa', '12', axis), '#aa0000');
  settings = setFactionColour(settings, 'all', 'cc', '5', null, '#0000aa');
  assert.equal(settings.campaigns.cc.factions['5'].colour, '#0000aa');
  assert.equal(setFactionColour(settings, 'all', 'aa', '12', axis, 'nope'), settings);
  settings = setFactionColour(settings, 'all', 'aa', '12', axis, null);
  assert.equal(chosenColour(settings, 'aa', '12', axis), undefined);
  const storage = memory();
  saveCampaignSettings(setFactionColour(settings, 'all', 'aa', '12', axis, '#abcdef'), storage);
  assert.deepEqual(loadCampaignSettings(storage).shared, { [axis!]: { colour: '#abcdef' } });
});

test('settings past the storage budget are refused on save, keeping what was saved before', () => {
  const storage = memory();
  assert.equal(saveCampaignSettings(setCampaignFlag(parseCampaignSettings(undefined), 'aa', 'GER'), storage), null);
  const large = setCampaignFlag(parseCampaignSettings(undefined), 'bb', 'x'.repeat(MAX_CAMPAIGN_SETTINGS_STORAGE));
  assert.notEqual(saveCampaignSettings(large, storage), null);
  assert.deepEqual(loadCampaignSettings(storage).campaigns, { aa: { factions: {}, flag: 'GER' } });
});

test('every campaign keeps its faction settings through save and reload, however many there are', () => {
  let settings = parseCampaignSettings(undefined);
  for (let id = 0; id < 300; id++) settings = setFactionStyle(settings, id.toString(16), '1', { colour: '#112233' });
  settings = setFactionStyle(settings, 'ffff', '1', { name: 'Late' });
  const storage = memory();
  assert.equal(saveCampaignSettings(settings, storage), null);
  const loaded = loadCampaignSettings(storage);
  assert.equal(Object.keys(loaded.campaigns).length, 301);
  assert.deepEqual(loaded.campaigns.ffff.factions, { 1: { name: 'Late' } });
});

test('every faction keeps its colour through save and reload, however many a campaign has', () => {
  const factions = Array.from({ length: 1500 }, (_, id) => ({ id: String(id), name: `Faction ${id}` }));
  let settings = parseCampaignSettings(undefined);
  for (const faction of factions) settings = setFactionColour(settings, 'campaign', 'aa', faction.id, null, '#112233');
  for (const faction of factions) settings = setFactionColour(settings, 'all', 'bb', faction.id, factionIdentity(faction.name, null), '#445566');
  const storage = memory();
  assert.equal(saveCampaignSettings(settings, storage), null);
  const loaded = loadCampaignSettings(storage);
  assert.equal(Object.keys(loaded.campaigns.aa.factions).length, 1500);
  assert.equal(Object.keys(loaded.shared).length, 1500);
});
