import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, FactionGroup } from '../generated/dashboard';
import { factionChoiceLabel, factionNameAt, factionNameLabel, latestFactionSummaries } from './factionNames.ts';

const faction: FactionGroup = {
  id: '9001', name: 'Later pact', nameReason: null, recordedName: '', firstHour: 10, lastHour: 40,
  names: [
    { firstHour: 10, lastHour: 19, name: 'Opening pact', reason: null, token: null },
    { firstHour: 25, lastHour: 29, name: null, reason: 'Faction template new_pact is not in the recorded content', token: null },
    { firstHour: 30, lastHour: 40, name: 'Later pact', reason: null, token: null },
  ], members: [], leaders: [], incomplete: [],
};

test('faction names follow the template read at an hour and never cross a gap', () => {
  assert.equal(factionNameLabel(faction, 10), 'Opening pact');
  assert.equal(factionNameLabel(faction, 19.9), 'Opening pact');
  assert.equal(factionNameLabel(faction, 30), 'Later pact');
  assert.equal(factionNameLabel(faction), 'Later pact');
  for (const hour of [9, 20, 24, 41, NaN]) {
    assert.deepEqual(factionNameAt(faction, hour), { name: null, reason: 'Not recorded' });
    assert.equal(factionNameLabel(faction, hour), 'Unnamed faction');
  }
});

test('a faction as a choice is named at the hour, else by its latest read, else unnamed', () => {
  assert.equal(factionChoiceLabel(faction, 10), 'Opening pact');
  for (const hour of [9, 25, 41, undefined]) assert.equal(factionChoiceLabel(faction, hour), 'Later pact');
  assert.equal(factionChoiceLabel(faction, 9, 'My pact'), 'My pact');
  assert.equal(factionChoiceLabel({ ...faction, name: null, nameReason: 'Template missing' }, 9), 'Unnamed faction');
});

test('missing or refused content keeps its exact reason and a display name wins', () => {
  assert.deepEqual(factionNameAt(faction, 25), { name: null, reason: 'Faction template new_pact is not in the recorded content' });
  for (const hour of [10, 20, 25, 30, undefined]) {
    assert.deepEqual(factionNameAt(faction, hour, 'My pact'), { name: 'My pact', reason: null });
  }
  const unavailable = { ...faction, name: null, nameReason: 'LexicalTokens not in this recording' };
  assert.deepEqual(factionNameAt(unavailable), { name: null, reason: 'LexicalTokens not in this recording' });
});


test('campaign summaries choose the newest branch read even when its name is unavailable', () => {
  const reply = (faction: FactionGroup): CountryGroupReply => ({ branchId: '1', factions: [faction], humans: [], wars: [],
    warStarts: [], capitulations: [], identityChanges: [], countryIdentities: [], unobserved: { humans: [], factions: [], wars: [], unattributed: [] }, warnings: [] });
  const first = { ...faction, lastHour: 19, names: [faction.names[0]], name: 'Opening pact', recordedName: 'Original literal' };
  const latest = { ...faction, lastHour: 18, name: null, nameReason: 'Template missing', recordedName: '' };
  const [summary] = latestFactionSummaries([reply(first), reply(latest)]);
  assert.equal(summary.name, null);
  assert.equal(summary.nameReason, 'Template missing');
  assert.equal(summary.recordedName, 'Original literal');
  assert.equal(latestFactionSummaries([reply(latest), reply(first)])[0].name, null);
  assert.equal(first.name, 'Opening pact');
});
