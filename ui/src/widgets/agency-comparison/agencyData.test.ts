import assert from 'node:assert/strict';
import test from 'node:test';
import type { AgencyComparisonReply, AgencyValue } from '../../generated/dashboard';
import { agencyRequestParams, agencyRows, agencyTooltip, operativesCell, upgradesCell, validateAgencyComparisonReply } from './agencyData.ts';
import { sortRows } from '../../charts/table/logic.ts';

const value = (created = true): AgencyValue => ({ created, name: created ? 'Recorded agency' : 'Agence par défaut', defenseRaw: '125000', defenseText: 'Defence level: 1.25', defenseValue: '1.25', activeCount: 3, maximum: 5,
  upgrades: { count: null, items: [] } });
const fixture = (): AgencyComparisonReply => ({ branchId: '1', available: true, defenseLabel: 'Defence level', observed: [{ firstHour: 10, lastHour: 20 }, { firstHour: 25, lastHour: 30 }],
  upgradesUnrecorded: 'Acquired upgrade levels were not recorded; elapsed work is a different field.', operativeReadHours: [10, 17, 25], warnings: [],
  series: [{ tag: 'CUSTOM', segments: [{ firstHour: 10, lastHour: 30 }], membership: null,
  rosters: [{ firstHour: 10, lastHour: 20, count: 2 }, { firstHour: 25, lastHour: 30, count: 0 }],
  entries: [{ firstHour: 10, lastHour: 20, readHour: 10, value: value() }, { firstHour: 25, lastHour: 30, readHour: 25, value: value() }] }] });

test('agency gaps and absent rows stay missing; a recorded zero remains zero', () => {
  const reply = validateAgencyComparisonReply(fixture());
  assert.equal(agencyRows(reply, 21, null).length, 0);
  assert.equal(agencyRows(reply, 9, null).length, 0);
  reply.series[0].entries[1].firstHour = 27;
  assert.equal(agencyRows(reply, 25, null).length, 0);
  reply.series[0].entries[1].value.activeCount = 0;
  assert.equal(operativesCell(agencyRows(reply, 27, null)[0].value), '0 / 5');
  const crossed = fixture(); crossed.series[0].entries[0].lastHour = 24;
  assert.throws(() => validateAgencyComparisonReply(crossed), /unobserved/);
});

test('content defence label and native numeric text stay separate from the complete tooltip', () => {
  const reply = fixture();
  reply.defenseLabel = 'Protection spéciale';
  reply.series[0].entries[0].value.defenseValue = '1.2%';
  reply.series[0].entries[0].value.defenseText = 'Protection spéciale : 1.2%';
  const checked = validateAgencyComparisonReply(reply);
  const row = agencyRows(checked, 12, null)[0];
  assert.equal(checked.defenseLabel, 'Protection spéciale');
  assert.equal(row.value.defenseValue, '1.2%');
  assert.equal(agencyTooltip(row, 'Country', null).rows.find(row => row.label === 'Defence')?.value, 'Protection spéciale : 1.2%');
  reply.defenseLabel = null;
  row.value.defenseValue = null;
  assert.equal(validateAgencyComparisonReply(reply).defenseLabel, null);
  assert.equal(agencyRows(reply, 12, null)[0].value.defenseValue, null);
  const malformed = structuredClone(reply) as unknown as { defenseLabel?: string | null };
  delete malformed.defenseLabel;
  assert.throws(() => validateAgencyComparisonReply(malformed), /malformed/);
});

test('never-created agency keeps the content-localised name; upgrades are never elapsed work', () => {
  const row = { key: 'CUSTOM', tag: 'CUSTOM', hour: 10, value: value(false), operativeRosterCount: 2, operativeReadHour: 10 };
  assert.equal(agencyTooltip(row, 'Country at this hour', fixture().upgradesUnrecorded).subtitle, 'Agence par défaut');
  assert.equal(upgradesCell(row.value), 'Not recorded');
  assert.equal(operativesCell(row.value), '3 / 5');
  const tooltip = agencyTooltip(row, 'Country at this hour', fixture().upgradesUnrecorded);
  assert.equal(tooltip.rows.find(row => row.label === 'Defence')?.value, 'Defence level: 1.25');
  assert.equal(tooltip.rows.filter(row => row.label === fixture().upgradesUnrecorded).length, 1);
  row.value.name = '';
  assert.equal(agencyTooltip(row, 'Country at this hour', null).subtitle, '');
});

test('agency upgrade identity retains its branch and a future recorded count hides the unrecorded reason', () => {
  const reply = fixture();
  const agency = reply.series[0].entries[0].value;
  agency.upgrades = { count: 2, items: [{ branchToken: 'mod_branch', token: 'mod_upgrade', name: null, icon: null }] };
  validateAgencyComparisonReply(reply);
  const tooltip = agencyTooltip({ key: 'CUSTOM', tag: 'CUSTOM', hour: 10, value: agency, operativeRosterCount: null, operativeReadHour: null }, 'Country', reply.upgradesUnrecorded);
  assert.equal(tooltip.rows.some(row => row.label === reply.upgradesUnrecorded), false);
  assert.equal(tooltip.rows.some(row => row.label === 'Unknown upgrade (mod_branch/mod_upgrade)'), true);
  const malformed = structuredClone(reply) as unknown as { series: { entries: { value: { upgrades: { items: { branchToken?: string }[] } } }[] }[] };
  delete malformed.series[0].entries[0].value.upgrades.items[0].branchToken;
  assert.throws(() => validateAgencyComparisonReply(malformed), /malformed/);
});

test('agency products retain all 5000 countries and backend top membership, with exact sort', () => {
  const reply = fixture();
  reply.series = Array.from({ length: 5000 }, (_, i) => ({ tag: `Synthetic_${i}`, segments: [{ firstHour: 10, lastHour: 30 }], membership: null,
    rosters: [{ firstHour: 10, lastHour: 20, count: 2 }],
    entries: [{ firstHour: 10, lastHour: 20, readHour: 10, value: { ...value(), defenseRaw: String(BigInt(i) * 100000n) } }] }));
  const checked = validateAgencyComparisonReply(reply);
  const rows = agencyRows(checked, 17, null);
  assert.equal(rows.length, 5000);
  assert.equal(rows[0].hour, 17);
  assert.equal(sortRows(rows, row => BigInt(row.value.defenseRaw), 'descending', row => row.key)[0].tag, 'Synthetic_4999');
  assert.deepEqual(agencyRows(checked, 17, new Set(['Synthetic_4321'])).map(row => row.tag), ['Synthetic_4321']);
  reply.series[0].membership = [{ firstHour: 18, lastHour: 20 }];
  assert.equal(agencyRows(reply, 17, null).length, 4999);
});

test('equal roster counts keep one run with exact later read dates; unavailable roster hours stay unknown', () => {
  const reply = validateAgencyComparisonReply(fixture());
  assert.equal(reply.series[0].entries.length, 2);
  assert.deepEqual(agencyRows(reply, 16, null).map(row => [row.operativeRosterCount, row.operativeReadHour]), [[2, 10]]);
  assert.deepEqual(agencyRows(reply, 17, null).map(row => [row.operativeRosterCount, row.operativeReadHour]), [[2, 17]]);
  assert.deepEqual(agencyRows(reply, 27, null).map(row => [row.operativeRosterCount, row.operativeReadHour]), [[0, 25]]);
  reply.series[0].rosters[0].lastHour = 18;
  assert.deepEqual(agencyRows(reply, 19, null).map(row => [row.operativeRosterCount, row.operativeReadHour]), [[null, null]]);
  const undated = fixture(); undated.operativeReadHours = [17, 25];
  assert.throws(() => validateAgencyComparisonReply(undated), /undated/);
  const unknownIdentity = fixture(); unknownIdentity.series[0].segments = [{ firstHour: 10, lastHour: 15 }, { firstHour: 17, lastHour: 30 }];
  assert.throws(() => validateAgencyComparisonReply(unknownIdentity), /unresolved roster/);
});

test('selected agency request is bounded to its countries; a missing selection cannot request all', () => {
  assert.deepEqual(agencyRequestParams('1', { kind: 'tags', tags: ['Chosen mod country'] }, [], undefined, 30, false, 'defense'),
    { branchId: '1', tags: ['Chosen mod country'], includeUncreated: false, top: null });
  assert.equal(agencyRequestParams('1', { kind: 'selected' }, [], undefined, 30, false, 'defense'), null);
  assert.deepEqual(agencyRequestParams('1', { kind: 'all' }, [], undefined, 30, true, 'active')?.tags, []);
  assert.deepEqual(agencyRequestParams('1', { kind: 'group', group: { type: 'top', count: 5000 }, membership: 'hour', aggregate: false }, [], undefined, 30, false, 'maximum')?.top,
    { count: 5000, field: 'maximum' });
});
