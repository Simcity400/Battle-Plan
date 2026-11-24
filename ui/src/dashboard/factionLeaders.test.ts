import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, FactionGroup } from '../generated/dashboard';
import { factionFounders, founderReads, leaderColours } from './factionLeaders.ts';
import { factionColours } from '../charts/factionColours.ts';

const run = (tag: string, firstHour: number, lastHour: number) => ({ tag, firstHour, lastHour });
const faction = (id: string, leaders: FactionGroup['leaders']): FactionGroup => ({ id, name: null, recordedName: '', firstHour: 0, lastHour: 999, nameReason: 'Faction template not recorded', names: [{ firstHour: 0, lastHour: 999, name: null, reason: 'Faction template not recorded', token: null }], members: [], leaders, incomplete: [] });
const reply = (branchId: string, factions: FactionGroup[]): CountryGroupReply => ({
  branchId, humans: [], factions, wars: [], warStarts: [], capitulations: [], identityChanges: [], countryIdentities: [],
  unobserved: { humans: [], factions: [], wars: [], unattributed: [] }, warnings: [],
});

test('a faction keeps the colour of the leader it had when first recorded, over every branch', () => {
  const founders = factionFounders([
    reply('1', [faction('7', [run('HUN', 300, 400), run('D01', 100, 299)]), faction('8', [])]),
    reply('1', [faction('9', [run('MY TAG', 500, 600)])]),
    reply('2', [faction('9', [run('A:B', 50, 60)]), faction('7', [run('ITA', 100, 200)])]),
  ]);
  assert.deepEqual(founders.get('7'), { branchId: '1', hour: 100, tag: 'D01' });
  assert.deepEqual(founders.get('9'), { branchId: '2', hour: 50, tag: 'A:B' });
  assert.equal(founders.has('8'), false);
  assert.deepEqual([...founderReads(founders)], [['1', [{ hour: 100, tag: 'D01' }]], ['2', [{ hour: 50, tag: 'A:B' }]]]);
});

test('founders read their recorded map colour, and a colour not recorded falls back to the palette', () => {
  const founders = new Map([['7', { branchId: '1', hour: 100, tag: 'D01' }], ['9', { branchId: '2', hour: 50, tag: 'A:B' }]]);
  const styles = new Map([['1:100:D01', { color: [200, 40, 40] as [number, number, number] }], ['2:50:A:B', { color: null }]]);
  const colours = leaderColours(founders, founder => styles.get(`${founder.branchId}:${founder.hour}:${founder.tag}`));
  assert.deepEqual(colours.get('7'), { r: 200, g: 40, b: 40 });
  assert.equal(colours.get('9'), null);
  const shown = factionColours([...colours].map(([id, leaderColor]) => ({ id, leaderColor })), {}, '#16181d');
  assert.match(shown.get('7')!, /^#[0-9a-f]{6}$/);
  assert.notEqual(shown.get('7'), shown.get('9'));
  assert.equal(factionColours([...colours].map(([id, leaderColor]) => ({ id, leaderColor })), { 7: '#123456' }, '#16181d').get('7'), '#123456');
});
