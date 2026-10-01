import test from 'node:test';
import assert from 'node:assert/strict';
import { applyColourSet, chosenColour, factionIdentity, loadCampaignSettings, MAX_FACTIONS, parseCampaignSettings, saveCampaignSettings, saveColourSet,
  setFactionColour, setFactionStyle } from './campaignSettings.ts';
import { FACTION_APART, factionColours } from '../charts/factionColours.ts';
import { parseColor, rgbToOklch } from '../shell/appearance/color.ts';
import { categoricalPalette, distance } from '../charts/palette.ts';

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
  assert.deepEqual(parseCampaignSettings({ version: 2 }), { version: 1, campaigns: {}, shared: {}, sets: [] });
  // Settings saved before colours for every campaign existed still load.
  assert.deepEqual(parseCampaignSettings({ version: 1, campaigns: {}, sets: [] }).shared, {});
  assert.deepEqual(parseCampaignSettings({ version: 1, campaigns: {}, sets: [], shared: { '["name","axis"]': { colour: '#AA0000' }, x: { colour: 'nope' }, __proto__: { colour: '#000000' } } }).shared,
    { '["name","axis"]': { colour: '#aa0000' } });
});

test('colour sets save by faction name and apply to another campaign, ignoring case', () => {
  let settings = setFactionStyle(parseCampaignSettings(undefined), 'aa', '12', { colour: '#222222', name: 'The Axis' });
  settings = saveColourSet(settings, 'mp', 'MP group', 'aa', FACTIONS, new Map([['3', '#3366cc'], ['12', '#999999']]));
  assert.deepEqual(settings.sets[0].entries, [{ faction: 'Axis', colour: '#222222', name: 'The Axis' }, { faction: 'Allies', colour: '#3366cc' }]);
  const { settings: applied, matched } = applyColourSet(settings, settings.sets[0], 'bb', [{ id: '7', name: 'AXIS' }, { id: '8', name: 'Comintern' }]);
  assert.equal(matched, 1);
  assert.deepEqual(applied.campaigns.bb.factions, { 7: { colour: '#222222', name: 'The Axis' } });
});

test('default faction colours are the leader colour exactly, grey included, else the palette, and stable', () => {
  const surface = '#16181d';
  const factions = [{ id: '12', leaderColor: parseColor('#808080') }, { id: '3', leaderColor: null }, { id: '40', leaderColor: parseColor('#3a6ea5') }];
  const colours = factionColours(factions, {}, surface);
  assert.deepEqual(factionColours([...factions].reverse(), {}, surface), colours);
  // The user's decision: GER's grey stays grey, exactly as recorded.
  assert.equal(colours.get('12'), '#808080');
  assert.equal(colours.get('40'), '#3a6ea5');
  // No recorded leader colour: the palette.
  assert.equal(colours.get('3'), categoricalPalette(surface)[0]);
  // A user's choice wins.
  assert.equal(factionColours(factions, { 40: '#123456' }, surface).get('40'), '#123456');
});

test('only clashing default colours are nudged, slightly, by lightness, a grey staying grey', () => {
  const surface = '#16181d';
  // Two factions led by countries of one grey, and one of a near-identical blue.
  const colours = factionColours([
    { id: '1', leaderColor: parseColor('#808080') }, { id: '2', leaderColor: parseColor('#808080') },
    { id: '3', leaderColor: parseColor('#3a6ea5') }, { id: '4', leaderColor: parseColor('#3b6fa6') },
  ], {}, surface);
  assert.equal(colours.get('1'), '#808080');
  assert.equal(colours.get('3'), '#3a6ea5');
  const [first, second, blue, nearBlue] = ['1', '2', '3', '4'].map(id => parseColor(colours.get(id)!)!);
  // The later one moved just far enough to be told apart, and no further than it needed.
  assert.ok(distance(first, second) >= FACTION_APART && distance(first, second) < FACTION_APART + 2);
  assert.ok(distance(blue, nearBlue) >= FACTION_APART && distance(blue, nearBlue) < FACTION_APART + 2);
  // Still a grey, never turned into a hue; the blue keeps its hue.
  assert.ok(rgbToOklch(second).c < 0.01);
  assert.ok(Math.abs(rgbToOklch(nearBlue).h - rgbToOklch(blue).h) < 3);
  // A default clashing with a user's choice moves; the choice never does.
  const around = factionColours([{ id: '1', leaderColor: parseColor('#808080') }, { id: '2' }], { 2: '#808080' }, surface);
  assert.equal(around.get('2'), '#808080');
  assert.ok(distance(parseColor(around.get('1')!)!, parseColor('#808080')!) >= FACTION_APART);
});

test('a colour set for all campaigns applies wherever the faction is found; a campaign can override it', () => {
  const axis = factionIdentity('Axis', 'GER');
  // Named factions match by their name as recorded (a key or literal, never a translation);
  // unnamed ones by their first leader.
  assert.equal(axis, factionIdentity('Axis', 'ITA'));
  assert.notEqual(factionIdentity('FACTION_PACT', null), factionIdentity('The Pact', null));
  assert.equal(factionIdentity(null, 'D01'), '["leader","D01"]');
  assert.equal(factionIdentity(null, null), null);
  let settings = parseCampaignSettings(undefined);
  settings = setFactionStyle(settings, 'aa', '12', { colour: '#111111', name: 'The Axis' });
  // Set for every campaign from campaign aa: aa's own colour gives way, its name stays.
  settings = setFactionColour(settings, 'all', 'aa', '12', axis, '#AA0000');
  assert.deepEqual(settings.shared, { [axis!]: { colour: '#aa0000' } });
  assert.deepEqual(settings.campaigns.aa.factions['12'], { name: 'The Axis' });
  assert.equal(chosenColour(settings, 'aa', '12', axis), '#aa0000');
  // Another campaign, whose faction has another id, finds it by identity.
  assert.equal(chosenColour(settings, 'bb', '99', axis), '#aa0000');
  // This campaign only: an override there, the other campaigns keep the shared colour.
  settings = setFactionColour(settings, 'campaign', 'bb', '99', axis, '#00aa00');
  assert.equal(chosenColour(settings, 'bb', '99', axis), '#00aa00');
  assert.equal(chosenColour(settings, 'aa', '12', axis), '#aa0000');
  // A faction with no identity can only be set for its campaign.
  settings = setFactionColour(settings, 'all', 'cc', '5', null, '#0000aa');
  assert.equal(settings.campaigns.cc.factions['5'].colour, '#0000aa');
  // Clearing the shared colour leaves the default again, and a malformed colour changes nothing.
  assert.equal(setFactionColour(settings, 'all', 'aa', '12', axis, 'nope'), settings);
  settings = setFactionColour(settings, 'all', 'aa', '12', axis, null);
  assert.equal(chosenColour(settings, 'aa', '12', axis), undefined);
  const storage = memory();
  saveCampaignSettings(setFactionColour(settings, 'all', 'aa', '12', axis, '#abcdef'), storage);
  assert.deepEqual(loadCampaignSettings(storage).shared, { [axis!]: { colour: '#abcdef' } });
});

test('a new faction style past the limit is refused, not dropped on save', () => {
  let settings = parseCampaignSettings(undefined);
  for (let id = 0; id < MAX_FACTIONS; id++) settings = setFactionStyle(settings, 'aa', String(id), { colour: '#112233' });
  assert.equal(setFactionStyle(settings, 'aa', '9999', { colour: '#112233' }), settings);
  assert.notEqual(setFactionStyle(settings, 'aa', '5', { colour: '#445566' }), settings);
});
