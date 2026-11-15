import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agencyRows, agencyStructure, agencyTooltip, upgradesCell, validateAgencies } from './agencyData.ts';
import type { Agency, AgencyComparisonReply } from '../../generated/dashboard';

const agency = (tag: string, defenseRaw: string, activeCount: number, maximum: number, sum: string | null, created = true): Agency => ({
  country: { id: tag.charCodeAt(0) * 100 + tag.charCodeAt(1), tag }, created, name: created ? `${tag} bureau` : 'Intelligence Agency', nameMissing: null,
  defenseRaw, defenseText: `Defense: ${Number(defenseRaw) / 100_000}`, defenseValue: String(Number(defenseRaw) / 100_000), activeCount, maximum,
  upgrades: sum === null ? null : { unavailable: null, nonnegativeLevelSum: sum, items: [{ ordinal: 0, branch: 'b', token: 't', level: Number(sum), name: 'Upgrade', missing: null }] },
});
const reply = (agencies: Agency[]): AgencyComparisonReply => ({ agencies, defenseLabel: 'Defense', upgradesMissing: null, missing: null, warnings: [] });

test('Top N ranks every agency by the chosen measure, ties by country; other selections keep the matching countries at the hour', () => {
  const agencies = reply([agency('ENG', '150000', 2, 5, '1'), agency('GER', '250000', 1, 3, '4'), agency('ITA', '150000', 5, 4, null)]);
  const every = () => true;
  assert.deepEqual(agencyRows(agencies, 0, every, true, { count: 2, field: 'defense' }).map(row => row.country.tag), ['GER', 'ENG']);
  assert.deepEqual(agencyRows(agencies, 0, every, true, { count: 1, field: 'active' }).map(row => row.country.tag), ['ITA']);
  assert.deepEqual(agencyRows(agencies, 0, every, true, { count: 1, field: 'maximum' }).map(row => row.country.tag), ['ENG']);
  assert.deepEqual(agencyRows(agencies, 7, (tag, hour) => hour === 7 && tag !== 'GER', false, null).map(row => row.country.tag), ['ENG', 'ITA']);
});

test('upgrades not recorded and a sum not known say so, never zero', () => {
  assert.equal(upgradesCell(agency('ITA', '0', 0, 0, null)), 'Not recorded');
  const unknown = { ...agency('ITA', '0', 0, 0, '0'), upgrades: { unavailable: 'Acquired agency levels contain unauthenticated identities', nonnegativeLevelSum: null, items: [] } };
  assert.equal(upgradesCell(unknown), 'Unknown');
  assert.equal(agencyTooltip(unknown, 'Italy', null).note, 'Acquired agency levels contain unauthenticated identities');
  assert.equal(agencyTooltip(agency('ITA', '0', 0, 0, null), 'Italy', 'AgencyUpgradeLevels is not recorded').note, 'AgencyUpgradeLevels is not recorded');
  assert.deepEqual(agencyTooltip(agency('GER', '250000', 1, 3, '4'), 'Germany', null).rows.at(-1), { label: 'Upgrade', value: 'Level 4' });
});

test('the reply is checked whole: a sum beside its refusal, or a name beside its reason, is malformed', () => {
  const good = reply([agency('GER', '250000', 1, 3, '4')]);
  assert.equal(validateAgencies(good), good);
  assert.throws(() => validateAgencies(reply([{ ...agency('GER', '1', 1, 1, '4'), upgrades: { unavailable: 'x', nonnegativeLevelSum: '4', items: [] } }])));
  assert.throws(() => validateAgencies(reply([{ ...agency('GER', '1', 1, 1, '1'), defenseRaw: '1.5' }])));
  assert.notEqual(agencyStructure(good), agencyStructure(reply([agency('GER', '250000', 2, 3, '4')])), 'an operative recruited renders');
});
