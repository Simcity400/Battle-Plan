import assert from 'node:assert/strict';
import test from 'node:test';
import type { AgencyComparisonReply, AgencyValue } from '../../generated/dashboard';
import { agencyRequestParams, agencyRows, agencyTooltip, operativesCell, upgradesCell, validateAgencyComparisonReply } from './agencyData.ts';
import { sortRows } from '../../charts/table/logic.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';

const value = (created = true): AgencyValue => ({ created, name: created ? 'Recorded agency' : 'Agence par défaut', defenseRaw: '125000', defenseText: 'Defence level: 1.25', defenseValue: '1.25', activeCount: 3, maximum: 5,
  upgrades: { readHour: null, unavailable: null, nonnegativeLevelSum: null, items: [] } });
const fixture = (): AgencyComparisonReply => ({ branchId: '1', available: true, defenseLabel: 'Defence level', observed: [{ firstHour: 10, lastHour: 20 }, { firstHour: 25, lastHour: 30 }],
  upgradesUnrecorded: 'Acquired upgrade levels were not recorded; elapsed work is a different field.', operativeReadHours: [10, 17, 25], warnings: [],
  series: [{ tag: 'CUSTOM', segments: [{ firstHour: 10, lastHour: 30 }], membership: null,
  rosters: [{ firstHour: 10, lastHour: 20, readHour: 10, count: 2 }, { firstHour: 25, lastHour: 30, readHour: 25, count: 0 }],
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

test('agency upgrade identity retains its branch and a recorded level sum hides the unrecorded reason', () => {
  const reply = fixture();
  const agency = reply.series[0].entries[0].value;
  agency.upgrades = { readHour: 10, unavailable: null, nonnegativeLevelSum: '2', items: [{ ordinal: 0, definitionPointer: '18446744073709551615', branchPointer: '9007199254740993', branchToken: 'mod_branch', token: 'mod_upgrade', branchTokenId: 0, tokenId: 0xffff_ffff, level: 2, identityStatus: 0, identityRefusal: null, name: null, contentReason: null, icon: null }] };
  validateAgencyComparisonReply(reply);
  const tooltip = agencyTooltip({ key: 'CUSTOM', tag: 'CUSTOM', hour: 10, value: agency, operativeRosterCount: null, operativeReadHour: null }, 'Country', reply.upgradesUnrecorded);
  assert.equal(tooltip.rows.some(row => row.label === reply.upgradesUnrecorded), false);
  assert.equal(tooltip.rows.find(row => row.label === 'Upgrade name unavailable')?.value, 'Level 2');
  assert.equal(tooltip.rows.some(row => row.label === 'Agency-upgrade read'), true);
  const malformed = structuredClone(reply) as unknown as { series: { entries: { value: { upgrades: { items: { branchToken?: string }[] } } }[] }[] };
  delete malformed.series[0].entries[0].value.upgrades.items[0].branchToken;
  assert.throws(() => validateAgencyComparisonReply(malformed), /malformed/);
});

test('agency acquired levels preserve repeated definitions, signed levels and refused identities', () => {
  const reply = fixture();
  const agency = reply.series[0].entries[0].value;
  const item = { ordinal: 0, definitionPointer: '9007199254740993', branchPointer: '9007199254740995',
    branchToken: 'mod_branch', token: 'mod_upgrade', branchTokenId: 12, tokenId: 34,
    level: 0, identityStatus: 0, identityRefusal: null, name: null, contentReason: null, icon: null };
  agency.upgrades = { readHour: 10, unavailable: null, nonnegativeLevelSum: null, items: [item, { ...item, ordinal: 1, level: -3 },
    { ...item, ordinal: 2, level: 7, branchToken: null, token: null, branchTokenId: null, tokenId: null,
      identityStatus: 14, identityRefusal: 'agency upgrade definition pointer repeated in database branch membership' }] };
  const checked = validateAgencyComparisonReply(reply);
  const row = agencyRows(checked, 12, null)[0];
  assert.equal(upgradesCell(row.value), 'Unknown');
  const tooltip = agencyTooltip(row, 'Country', reply.upgradesUnrecorded);
  assert.deepEqual(tooltip.rows.filter(row => row.value?.startsWith('Level ')).map(row => row.value), ['Level 0', 'Level -3', 'Level 7']);
  assert.equal(tooltip.rows.some(row => row.label === agency.upgrades.items[2].identityRefusal), true);
  assert.equal(tooltip.rows.some(row => row.label === reply.upgradesUnrecorded), false);
  assert.equal(checked.series[0].entries[0].value.upgrades.items[0].definitionPointer, '9007199254740993');
  const duplicateOrdinal = structuredClone(reply);
  duplicateOrdinal.series[0].entries[0].value.upgrades.items[1].ordinal = 0;
  assert.throws(() => validateAgencyComparisonReply(duplicateOrdinal), /malformed/);
  const refusedCount = structuredClone(reply);
  refusedCount.series[0].entries[0].value.upgrades.nonnegativeLevelSum = '4';
  assert.throws(() => validateAgencyComparisonReply(refusedCount), /malformed/);
  const undated = structuredClone(reply);
  undated.series[0].entries[0].value.upgrades.readHour = null;
  assert.throws(() => validateAgencyComparisonReply(undated), /malformed/);
  const lateRead = structuredClone(reply);
  lateRead.series[0].entries[0].value.upgrades.readHour = 11;
  assert.throws(() => validateAgencyComparisonReply(lateRead), /malformed/);
});

test('a failed acquired-list read is unknown with its recording-gap reason', () => {
  const reply = fixture();
  const agency = reply.series[0].entries[0].value;
  agency.upgrades.unavailable = 'AgencyUpgradeLevels read failed; acquired levels are unknown.';
  const row = agencyRows(validateAgencyComparisonReply(reply), 12, null)[0];
  assert.equal(upgradesCell(row.value), 'Unknown');
  const tooltip = agencyTooltip(row, 'Country', reply.upgradesUnrecorded);
  assert.equal(tooltip.rows.some(row => row.label === agency.upgrades.unavailable), true);
  assert.equal(tooltip.rows.some(row => row.label === reply.upgradesUnrecorded), false);
  const unavailableCount = structuredClone(reply);
  unavailableCount.series[0].entries[0].value.upgrades.nonnegativeLevelSum = '0';
  assert.throws(() => validateAgencyComparisonReply(unavailableCount), /malformed/);
});

test('agency level sums exceed signed totals and keep content refusals', () => {
  const reply = fixture();
  const agency = reply.series[0].entries[0].value;
  const item = { ordinal: 0, definitionPointer: '1', branchPointer: '2', branchToken: 'mod_branch', token: 'mod_upgrade',
    branchTokenId: 1, tokenId: 2, level: 0x7fff_ffff, identityStatus: 0, identityRefusal: null,
    name: null, contentReason: 'agency upgrade mod_branch/mod_upgrade content definition unavailable', icon: null };
  agency.upgrades = { readHour: 10, unavailable: null, nonnegativeLevelSum: '4294967294',
    items: [item, { ...item, ordinal: 1 }, { ...item, ordinal: 2, level: -3, contentReason: null }] };
  const row = agencyRows(validateAgencyComparisonReply(reply), 12, null)[0];
  assert.equal(upgradesCell(row.value), '4,294,967,294');
  const tooltip = agencyTooltip(row, 'Country', reply.upgradesUnrecorded);
  assert.equal(tooltip.rows.some(row => row.label === item.contentReason), true);
  assert.match(tooltip.note ?? '', /inventory total/);
  for (const invalid of [-1, 4, '-1', '1.5', '', '0', '4294967293', '4294967295']) {
    const malformed = structuredClone(reply) as unknown as { series: { entries: { value: { upgrades: { nonnegativeLevelSum: unknown } } }[] }[] };
    malformed.series[0].entries[0].value.upgrades.nonnegativeLevelSum = invalid;
    assert.throws(() => validateAgencyComparisonReply(malformed), /malformed/);
  }
  const noContentReason = structuredClone(reply) as unknown as { series: { entries: { value: { upgrades: { items: { contentReason?: string | null }[] } } }[] }[] };
  delete noContentReason.series[0].entries[0].value.upgrades.items[0].contentReason;
  assert.throws(() => validateAgencyComparisonReply(noContentReason), /malformed/);
});

test('a recorded nonpositive acquired list has an exact zero sum', () => {
  const reply = fixture();
  const agency = reply.series[0].entries[0].value;
  const item = { ordinal: 0, definitionPointer: '1', branchPointer: '2', branchToken: 'mod_branch', token: 'mod_upgrade',
    branchTokenId: 1, tokenId: 2, level: 0, identityStatus: 0, identityRefusal: null, name: null, contentReason: null, icon: null };
  agency.upgrades = { readHour: 10, unavailable: null, nonnegativeLevelSum: '0', items: [item, { ...item, ordinal: 1, level: -3 }] };
  const row = agencyRows(validateAgencyComparisonReply(reply), 12, null)[0];
  assert.equal(upgradesCell(row.value), '0');
  agency.upgrades.nonnegativeLevelSum = '1';
  assert.throws(() => validateAgencyComparisonReply(reply), /malformed/);
});

test('agency products retain all 5000 countries and backend top membership, with exact sort', () => {
  const reply = fixture();
  reply.series = Array.from({ length: 5000 }, (_, i) => ({ tag: `Synthetic_${i}`, segments: [{ firstHour: 10, lastHour: 30 }], membership: null,
    rosters: [{ firstHour: 10, lastHour: 20, readHour: 10, count: 2 }],
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

test('an off-read alias keeps its actual census date without a visible source-alias scalar', () => {
  const reply: AgencyComparisonReply = { branchId: '1', available: true, defenseLabel: 'Defence level',
    observed: [{ firstHour: 15, lastHour: 22 }], upgradesUnrecorded: null,
    operativeReadHours: [15, 20], warnings: [], series: [{ tag: 'Y',
      segments: [{ firstHour: 16, lastHour: 22 }], membership: null,
      rosters: [{ firstHour: 16, lastHour: 21, readHour: 15, count: 2 }],
      entries: [{ firstHour: 16, lastHour: 22, readHour: 16, value: { ...value(), activeCount: 7 } }] }] };
  const checked = validateAgencyComparisonReply(reply);
  assert.deepEqual(checked.operativeReadHours, [15, 20]);
  assert.deepEqual(agencyRows(checked, 16, null).map(row => [row.tag, row.value.activeCount, row.operativeRosterCount, row.operativeReadHour]),
    [['Y', 7, 2, 15]]);
  for (const [hour, read] of [[16, 15], [20, 20]]) {
    const row = agencyRows(checked, hour, null)[0];
    assert.equal(agencyTooltip(row, 'Country', null).rows.find(item => item.label === 'Active-list read')?.value, formatDateTime(read));
  }
  assert.deepEqual(agencyRows(checked, 22, null).map(row => [row.value.activeCount, row.operativeRosterCount, row.operativeReadHour]), [[7, null, null]]);
  const clipped = structuredClone(reply);
  clipped.series[0].rosters[0].firstHour = 21;
  assert.deepEqual(agencyRows(validateAgencyComparisonReply(clipped), 21, null).map(row => [row.operativeRosterCount, row.operativeReadHour]), [[2, 20]]);
  const unlisted = structuredClone(reply); unlisted.series[0].rosters[0].readHour = 14;
  assert.throws(() => validateAgencyComparisonReply(unlisted), /undated/);
  const future = structuredClone(reply); future.series[0].rosters[0].readHour = 20;
  assert.throws(() => validateAgencyComparisonReply(future), /malformed/);
  const missing = structuredClone(reply) as unknown as { series: { rosters: { readHour?: number }[] }[] };
  delete missing.series[0].rosters[0].readHour;
  assert.throws(() => validateAgencyComparisonReply(missing), /malformed/);
});

test('selected agency request is bounded to its countries; a missing selection cannot request all', () => {
  const groups = { countryIdentities: [{ tag: 'Chosen mod country', identities: [{ firstHour: 10, lastHour: 30, slot: 1, countryId: 7 }] }] } as import('../../generated/dashboard').CountryGroupReply;
  assert.equal(agencyRequestParams('1', { kind: 'tags', tags: ['Chosen mod country'] }, [], undefined, 30, false, 'defense'), null);
  assert.deepEqual(agencyRequestParams('1', { kind: 'tags', tags: ['Chosen mod country'] }, [], groups, 30, false, 'defense'),
    { branchId: '1', tags: ['Chosen mod country'], includeUncreated: false, top: null });
  assert.equal(agencyRequestParams('1', { kind: 'selected' }, [], undefined, 30, false, 'defense'), null);
  assert.deepEqual(agencyRequestParams('1', { kind: 'all' }, [], undefined, 30, true, 'active')?.tags, []);
  assert.deepEqual(agencyRequestParams('1', { kind: 'group', group: { type: 'top', count: 5000 }, membership: 'hour', aggregate: false }, [], undefined, 30, false, 'maximum')?.top,
    { count: 5000, field: 'maximum' });
});
