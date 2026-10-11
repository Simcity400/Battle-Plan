import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { OperativeRosterReply } from '../../generated/dashboard';
import { resolveOptions } from '../../dashboard/options.ts';
import { sortRows } from '../../charts/table/logic.ts';
import { runAt } from '../../charts/series.ts';
import { countryMatch, resolveCountries } from '../../dashboard/groups.ts';
import { definition, DEFAULT_SORT, OPTIONS } from './definition.ts';
import { departureText, operativeLabelHour, operativeMoments, operativeName, operativeTooltip as flaggedOperativeTooltip, rosterAt, validateOperativesRosterReply } from './operativesRosterData.ts';
const operativeTooltip = (reply: OperativeRosterReply, row: Parameters<typeof flaggedOperativeTooltip>[1], shownHour: number, name: (tag: string, hour: number) => string) =>
  flaggedOperativeTooltip(reply, row, shownHour, (tag, hour) => ({ name: name(tag, hour) }));

const fixture = (): OperativeRosterReply => ({ branchId: '1', operations: [], agencyAbsent: [], artwork: ['data:image/png;base64,cG9ydHJhaXQ='], reasons: ['Portrait link not recorded', 'Mission art unavailable', 'Experience denominator unavailable', 'Assignment operands not recorded'], warnings: [], countries: [{ tag: 'MOD owner',
  identities: [{ firstHour: 100, lastHour: 199, slot: 1, countryId: 7 }, { firstHour: 220, lastHour: 300, slot: 1, countryId: 7 }],
  observed: [{ firstHour: 100, lastHour: 123, readHour: 100 }, { firstHour: 124, lastHour: 199, readHour: 124 },
    { firstHour: 220, lastHour: 300, readHour: 220 }],
  operatives: [{ key: 'operative:7:100', id: '7', joinedHour: 100, departedHour: null, departedReason: null, runs: [
    { firstHour: 100, lastHour: 123, value: { name: 'Literal operative name', unnamed: false, nameMissing: null, portrait: null, portraitMissing: 0, skill: 2,
      experienceRaw: '12500000', experienceText: '12.5%', experienceMissing: null, experienceRatio: 0.125, assignmentName: 'Content mission name', assignmentMissing: null,
      assignmentIcon: null, assignmentIconMissing: 1, assignmentOperation: false, assignmentDuration: null, assignmentProgress: null, targetTag: 'MOD old target', captured: false,
      capturerTag: null, capturedHour: null, ownerTag: 'MOD owner' } },
    { firstHour: 124, lastHour: 199, value: { name: 'Localised operative name', unnamed: false, nameMissing: null, portrait: null, portraitMissing: 0, skill: 3,
      experienceRaw: '30000000', experienceText: '30%', experienceMissing: null, experienceRatio: 0.3, assignmentName: 'Captured', assignmentMissing: null,
      assignmentIcon: null, assignmentIconMissing: null, assignmentOperation: false, assignmentDuration: null, assignmentProgress: null, targetTag: 'MOD old target', captured: true,
      capturerTag: 'MOD capturer', capturedHour: 119, ownerTag: 'MOD owner' } },
    { firstHour: 220, lastHour: 300, value: { name: null, unnamed: true, nameMissing: null, portrait: null, portraitMissing: 0, skill: null,
      experienceRaw: '30000000', experienceText: null, experienceMissing: 2, experienceRatio: null, assignmentName: null, assignmentMissing: 3,
      assignmentIcon: null, assignmentIconMissing: null, assignmentOperation: false, assignmentDuration: null, assignmentProgress: null, targetTag: 'MOD renamed target', captured: null,
      capturerTag: null, capturedHour: null, ownerTag: 'MOD owner' } },
  ] }],
}] });

test('operative country roles preserve their own dated flag metadata and a departed row never reads a later identity', () => {
  const reply = fixture();
  const entry = reply.countries[0].operatives[0];
  entry.runs = entry.runs.slice(0, 2);
  entry.departedHour = 205;
  const row = rosterAt(reply, 250, true).rows[0];
  const wanted: [string, number][] = [];
  const tip = flaggedOperativeTooltip(reply, row, 250, (tag, hour) => {
    wanted.push([tag, hour]);
    return { name: `${tag} at ${hour}`, flag: tag === 'MOD owner' ? 'old-owner-flag' : undefined,
      flagPending: tag === 'MOD old target', flagReason: tag === 'MOD capturer' ? 'Capturer flag failed at its read' : undefined };
  });
  assert.ok(wanted.length > 0 && wanted.every(([, hour]) => hour === 199));
  assert.equal(tip.rows.find(row => row.label === 'Country')?.countryFlag?.flag, 'old-owner-flag');
  assert.equal(tip.rows.find(row => row.label === 'Target')?.countryFlag?.flagPending, true);
  assert.equal(tip.rows.find(row => row.label === 'Capturer')?.countryFlag?.flagReason, 'Capturer flag failed at its read');
  assert.ok(tip.rows.filter(row => !['Country', 'Roster country', 'Owner', 'Target', 'Capturer'].includes(row.label)).every(row => row.countryFlag === undefined));
  row.run.value = { ...row.run.value, ownerTag: null, targetTag: null, capturerTag: null };
  const unknown = flaggedOperativeTooltip(reply, row, 250, () => ({ name: 'Roster', flag: 'must-not-use-for-unknown' }));
  assert.equal(unknown.rows.find(row => row.label === 'Owner')?.countryFlag, undefined);
  assert.equal(unknown.rows.find(row => row.label === 'Capturer')?.countryFlag, undefined);
  assert.equal(unknown.rows.find(row => row.label === 'Target'), undefined);
});

test('departed roster selection uses its last native holder date after unrelated tag reuse', () => {
  const reply = fixture();
  const entry = reply.countries[0].operatives[0];
  entry.runs = entry.runs.slice(0, 2);
  entry.departedHour = 205;
  const groups = { countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 250, lastHour: 300, slot: 1, countryId: 7 }] },
    { tag: 'MOD owner', identities: [{ firstHour: 100, lastHour: 199, slot: 1, countryId: 7 }, { firstHour: 220, lastHour: 300, slot: 2, countryId: 9 }] },
  ] } as import('../../generated/dashboard').CountryGroupReply;
  const match = countryMatch({ kind: 'tags', tags: ['X'] }, [], groups, 300);
  assert.equal(match.matches('MOD owner', 250), false);
  const view = rosterAt(reply, 250, true, match.matches);
  assert.equal(view.rows.length, 1);
  assert.equal(view.rows[0].departed, true);
  assert.equal(operativeLabelHour(view.rows[0], 250), 199);
  assert.deepEqual(view.unknown, []);
  assert.equal(rosterAt(reply, 250, false, match.matches).rows.length, 0);
});

test('daily roster reads stay exact and never cross recording gaps', () => {
  const reply = validateOperativesRosterReply(fixture());
  assert.equal(rosterAt(reply, 123, false).rows[0].run.value.experienceRaw, '12500000');
  assert.equal(rosterAt(reply, 124, false).rows[0].run.value.experienceRaw, '30000000');
  assert.equal(rosterAt(reply, 150, false).rows[0].readHour, 124);
  assert.deepEqual(rosterAt(reply, 200, true), { rows: [], unknown: ['MOD owner'] });
  assert.equal(rosterAt(reply, 99, false).rows.length, 0);
  assert.deepEqual(rosterAt(reply, 99, false).unknown, ['MOD owner']);
});

test('off-read aliases and return episodes retain their supplying census only with continuous native roster coverage', () => {
  const reply = fixture();
  const original = reply.countries[0];
  const entry = original.operatives[0];
  const run = entry.runs[0];
  reply.countries = [
    { tag: 'X', identities: [{ firstHour: 100, lastHour: 100, slot: 1, countryId: 7 },
      { firstHour: 102, lastHour: 102, slot: 1, countryId: 7 }],
      observed: [{ firstHour: 100, lastHour: 100, readHour: 100 }, { firstHour: 102, lastHour: 102, readHour: 100 }],
      operatives: [100, 102].map(hour => ({ ...entry, key: `X:${hour}`,
        runs: [{ ...run, firstHour: hour, lastHour: hour, value: { ...run.value, ownerTag: 'X' } }] })) },
    { tag: 'Y', identities: [{ firstHour: 101, lastHour: 101, slot: 1, countryId: 7 }],
      observed: [{ firstHour: 101, lastHour: 101, readHour: 100 }],
      operatives: [{ ...entry, key: 'Y:101', runs: [{ ...run, firstHour: 101, lastHour: 101,
        value: { ...run.value, ownerTag: 'Y' } }] }] },
  ];
  validateOperativesRosterReply(reply);
  for (const [hour, tag] of [[100, 'X'], [101, 'Y'], [102, 'X']] as const) {
    const row = rosterAt(reply, hour, false).rows[0];
    assert.equal(row.country, tag);
    assert.equal(row.readHour, 100);
    assert.equal(row.entry.joinedHour, 100);
  }
  const held = rosterAt(reply, 101, false).rows[0];
  assert.doesNotMatch(operativeTooltip(reply, held, 101, tag => tag).note ?? '', /Read /);
  const oldKey = rosterAt(reply, 100, false).rows[0].key;
  assert.notEqual(rosterAt(reply, 102, false).rows[0].key, oldKey);
  assert.deepEqual(rosterAt(reply, 103, false).rows, []);
  reply.countries[1].operatives[0].joinedHour = 101;
  assert.throws(() => validateOperativesRosterReply(reply), /invalid membership date/);
  reply.countries[1].operatives[0].joinedHour = 100;
  const proved = structuredClone(reply);
  // Native continuity cannot waive a missing supplying census/roster hour.
  reply.countries[1].observed = [];
  reply.countries[1].operatives = [];
  assert.throws(() => validateOperativesRosterReply(reply), /unproved held membership read/);
  reply.countries = structuredClone(proved.countries);
  reply.countries[1].identities[0].countryId = 9;
  assert.throws(() => validateOperativesRosterReply(reply), /unproved held membership read/);
  reply.countries = structuredClone(proved.countries);
  reply.countries[1].identities = [];
  assert.throws(() => validateOperativesRosterReply(reply), /unobserved native identities/);
  reply.countries = structuredClone(proved.countries);
  reply.countries.push({ ...structuredClone(reply.countries[1]), tag: 'Ambiguous alias' });
  assert.throws(() => validateOperativesRosterReply(reply), /unproved held membership read/);
  reply.countries = structuredClone(proved.countries);
  for (const country of reply.countries) country.operatives = [];
  validateOperativesRosterReply(reply);
  assert.equal(rosterAt(reply, 101, false).unknown.includes('Y'), false);
});

test('one equal value run takes its read date from each daily membership observation', () => {
  const reply = fixture();
  const country = reply.countries[0];
  country.observed = [{ firstHour: 100, lastHour: 123, readHour: 100 }, { firstHour: 124, lastHour: 147, readHour: 124 },
    { firstHour: 148, lastHour: 171, readHour: 148 }];
  const entry = country.operatives[0];
  entry.runs = [{ ...entry.runs[0], lastHour: 171 }];
  validateOperativesRosterReply(reply);
  for (const [hour, readHour] of [[123, 100], [124, 124], [150, 148]]) {
    const row = rosterAt(reply, hour, false).rows[0];
    assert.equal(row.readHour, readHour);
    const note = operativeTooltip(reply, row, hour, tag => tag).note ?? '';
    assert.doesNotMatch(note, /Read /);
  }
  entry.departedHour = 180;
  const departed = rosterAt(reply, 200, true).rows[0];
  assert.equal(departed.readHour, 148);
  assert.equal(operativeTooltip(reply, departed, 200, tag => tag).note ?? '', '');
});

test('literal, localised and empty names come from the dated value, never a later name', () => {
  const reply = fixture();
  assert.equal(operativeName(rosterAt(reply, 110, false).rows[0].run.value), 'Literal operative name');
  assert.equal(operativeName(rosterAt(reply, 124, false).rows[0].run.value), 'Localised operative name');
  const unknown = rosterAt(reply, 220, false).rows[0].run.value;
  assert.equal(operativeName(unknown), 'Unnamed operative');
  assert.equal(operativeName({ ...unknown, name: null, unnamed: false }), 'Name unknown');
});

test('a name unresolved without content stays unknown, while an empty name is unnamed', () => {
  const reply = fixture();
  const value = reply.countries[0].operatives[0].runs[0].value;
  value.name = null;
  value.nameMissing = reply.reasons.push('The recorded content stack is unavailable; saved key versus literal cannot be established') - 1;
  const row = rosterAt(reply, 110, false).rows[0];
  assert.equal(operativeTooltip(reply, row, 110, tag => tag).title, 'Name unknown');
  assert.equal(operativeTooltip(reply, row, 110, tag => tag).note ?? '', '');
  value.name = null; value.unnamed = true; value.nameMissing = null;
  assert.equal(operativeTooltip(reply, row, 110, tag => tag).title, 'Unnamed operative');
  value.name = 'Content says empty'; value.unnamed = false;
  assert.equal(operativeName(value), 'Content says empty');
});

test('capture has the recorded capturer and exact date, while joins are observations', () => {
  const reply = fixture();
  const row = rosterAt(reply, 140, false).rows[0];
  assert.deepEqual(operativeMoments(row).map(moment => [moment.kind, moment.hour]), [['joined', 100], ['captured', 119]]);
  const tip = operativeTooltip(reply, row, 140, tag => tag);
  assert.equal(tip.rows.find(row => row.label === 'Capturer')?.value, 'MOD capturer');
  assert.match(tip.rows.find(row => row.label === 'Captured on')?.value ?? '', /119/);
  assert.match(tip.rows.find(row => row.label === 'Present')?.value ?? '', /^by /);
  assert.doesNotMatch(tip.note ?? '', /read once a day/);
});

test('past captures remain jumpable after release and later capture reads stay hidden', () => {
  const reply = fixture();
  assert.deepEqual(operativeMoments(rosterAt(reply, 110, false).rows[0]).map(moment => moment.kind), ['joined']);
  const row = rosterAt(reply, 240, false).rows[0];
  assert.deepEqual(operativeMoments(row).map(moment => [moment.kind, moment.hour]), [['joined', 100], ['captured', 119]]);
  row.entry.runs[2].value.captured = true;
  row.entry.runs[2].value.capturedHour = 119;
  assert.equal(operativeMoments(row).filter(moment => moment.kind === 'captured').length, 1);
});

test('a future stored capture date remains exact and known independently of capturer identity', () => {
  const reply = fixture();
  const value = reply.countries[0].operatives[0].runs[1].value;
  value.capturedHour = 500;
  value.capturerTag = null;
  validateOperativesRosterReply(reply);
  const row = rosterAt(reply, 140, false).rows[0];
  const tip = operativeTooltip(reply, row, 140, tag => tag);
  assert.equal(tip.rows.find(row => row.label === 'Capturer')?.value, 'Unknown');
  assert.equal(tip.rows.find(row => row.label === 'Captured on')?.value, 'Hour 500');
  assert.deepEqual(operativeMoments(row).map(moment => [moment.kind, moment.hour]), [['joined', 100], ['captured', 500]]);
});

test('departures between reads say by and only campaign view includes them muted', () => {
  const reply = fixture();
  const entry = reply.countries[0].operatives[0];
  entry.runs = entry.runs.slice(0, 2);
  entry.departedHour = 220;
  entry.departedReason = 'Left (reason not recorded)';
  validateOperativesRosterReply(reply);
  assert.equal(rosterAt(reply, 219, true).rows.length, 0);
  assert.equal(rosterAt(reply, 220, false).rows.length, 0);
  const row = rosterAt(reply, 220, true).rows[0];
  assert.equal(row.departed, true);
  assert.equal(row.readHour, 124);
  assert.match(departureText(row) ?? '', /^Left by /);
  assert.deepEqual(operativeMoments(row).map(moment => [moment.kind, moment.hour]), [['joined', 100], ['captured', 119], ['departed', 220]]);
  assert.equal(operativeLabelHour(row, 300), 199);
});

test('killed first observed in departure read is excluded live and muted in campaign', () => {
  const reply = fixture();
  const entry = reply.countries[0].operatives[0];
  entry.runs = [{ ...entry.runs[0], firstHour: 100, lastHour: 100 }];
  entry.departedHour = 100;
  entry.departedReason = 'Killed';
  validateOperativesRosterReply(reply);
  assert.equal(rosterAt(reply, 100, false).rows.length, 0);
  assert.equal(rosterAt(reply, 100, true).rows[0].departed, true);
  assert.match(departureText(rosterAt(reply, 300, true).rows[0]) ?? '', /^Killed by /);
});

test('target identities use hovered hour for current rows and last projected hour for departed rows', () => {
  const reply = fixture();
  const label = (tag: string, hour: number) => `${tag} as of ${hour}`;
  const active = rosterAt(reply, 240, false).rows[0];
  assert.equal(operativeTooltip(reply, active, 240, label).rows.find(row => row.label === 'Target')?.value, 'MOD renamed target as of 240');
  const entry = reply.countries[0].operatives[0];
  entry.runs.pop(); entry.departedHour = 220;
  const departed = rosterAt(reply, 300, true).rows[0];
  assert.equal(operativeTooltip(reply, departed, 300, label).rows.find(row => row.label === 'Target')?.value, 'MOD old target as of 199');
  assert.equal(operativeTooltip(reply, departed, 300, label).note ?? '', '');
});

test('a final gap leaves current roster unknown while previously proven departures remain known', () => {
  const reply = fixture();
  const entry = reply.countries[0].operatives[0];
  entry.runs = entry.runs.slice(0, 1); entry.departedHour = 124; entry.departedReason = 'Killed';
  reply.countries[0].observed = reply.countries[0].observed.slice(0, 2);
  validateOperativesRosterReply(reply);
  const view = rosterAt(reply, 300, true);
  assert.deepEqual(view.unknown, ['MOD owner']);
  assert.equal(view.rows.length, 1);
  assert.equal(view.rows[0].departed, true);
  assert.equal(rosterAt(reply, 300, false).rows.length, 0);
});

test('missing owner stays unknown and is never replaced with the roster country', () => {
  const reply = fixture();
  reply.countries[0].operatives[0].runs[0].value.ownerTag = null;
  const tip = operativeTooltip(reply, rosterAt(reply, 120, false).rows[0], 120, tag => `named ${tag}`);
  assert.equal(tip.rows.find(row => row.label === 'Owner')?.value, 'Unknown');
  assert.equal(tip.rows.find(row => row.label === 'Roster country')?.value, 'named MOD owner');
});

test('unknown rosters remain distinct from known empty rosters', () => {
  const reply = fixture();
  reply.countries[0].operatives = [];
  assert.deepEqual(rosterAt(reply, 120, false), { rows: [], unknown: [] });
  reply.countries[0].observed = [];
  assert.deepEqual(rosterAt(reply, 120, false), { rows: [], unknown: ['MOD owner'] });
});

test('unknown skill, experience and assignment are said and native missing detail survives', () => {
  const reply = fixture();
  const row = rosterAt(reply, 220, false).rows[0];
  const tip = operativeTooltip(reply, row, 220, tag => tag);
  for (const label of ['Skill', 'Experience', 'Assignment', 'Captured']) assert.equal(tip.rows.find(row => row.label === label)?.value, 'Unknown');
  assert.equal(tip.note ?? '', '');
  assert.equal(row.run.value.experienceRatio, null);
});

test('operation definition read is dated independently of the membership read', () => {
  const reply = fixture();
  const value = reply.countries[0].operatives[0].runs[0].value;
  value.assignmentName = 'Content operation name'; value.assignmentOperation = true;
  reply.operations = [{ firstHour: 97, lastHour: 123, readHour: 97 }];
  const tip = operativeTooltip(reply, rosterAt(reply, 110, false).rows[0], 110, tag => tag);
  assert.equal(tip.rows.find(row => row.label === 'Assignment read')?.value, undefined);
  assert.doesNotMatch(tip.note ?? '', /read once a day/);
  reply.operations = [{ firstHour: 100, lastHour: 123, readHour: 100 }];
  assert.equal(operativeTooltip(reply, rosterAt(reply, 110, false).rows[0], 110, tag => tag).rows.some(row => row.label === 'Assignment read'), false);
});

test('proven agency feature absence is dated and never bridges unknown recording gaps', () => {
  const reply = fixture();
  reply.countries[0].observed = [];
  reply.countries[0].operatives = [];
  reply.agencyAbsent = [{ firstHour: 100, lastHour: 123 }, { firstHour: 148, lastHour: 171 }];
  validateOperativesRosterReply(reply);
  for (const hour of [100, 110, 123, 148, 171]) {
    assert.ok(runAt(reply.agencyAbsent, hour));
    assert.deepEqual(rosterAt(reply, hour, false), { rows: [], unknown: ['MOD owner'] });
  }
  for (const hour of [99, 124, 147, 172]) {
    assert.equal(runAt(reply.agencyAbsent, hour), null);
    assert.deepEqual(rosterAt(reply, hour, false), { rows: [], unknown: ['MOD owner'] });
  }
});

test('one equal operation run uses the current Operations read without repeating its value', () => {
  const reply = fixture();
  const country = reply.countries[0];
  country.observed = [{ firstHour: 100, lastHour: 123, readHour: 100 }, { firstHour: 124, lastHour: 147, readHour: 124 },
    { firstHour: 148, lastHour: 171, readHour: 148 }];
  reply.operations = [{ firstHour: 97, lastHour: 120, readHour: 97 }, { firstHour: 121, lastHour: 144, readHour: 121 },
    { firstHour: 145, lastHour: 171, readHour: 145 }, { firstHour: 172, lastHour: 195, readHour: 172 }];
  const entry = country.operatives[0];
  entry.runs = [{ ...entry.runs[0], lastHour: 171, value: { ...entry.runs[0].value, assignmentOperation: true } }];
  validateOperativesRosterReply(reply);
  for (const [hour, readHour, assignmentReadHour] of [[110, 100, 97], [130, 124, 121], [160, 148, 145]]) {
    const row = rosterAt(reply, hour, false).rows[0];
    assert.equal(row.readHour, readHour);
    assert.equal(row.assignmentReadHour, assignmentReadHour);
    assert.equal(row.run, entry.runs[0]);
    assert.equal(operativeTooltip(reply, row, hour, tag => tag).rows.find(row => row.label === 'Assignment read')?.value, undefined);
  }
  assert.equal(entry.runs.length, 1);
  entry.departedHour = 180;
  assert.equal(rosterAt(reply, 200, true).rows[0].assignmentReadHour, 145);
  reply.operations = [];
  assert.equal(rosterAt(reply, 200, true).rows[0].assignmentReadHour, null);
});

test('an Operations read dates admitted operation text or artwork and leaves missing definitions undated', () => {
  const reply = fixture();
  reply.operations = [{ firstHour: 97, lastHour: 123, readHour: 97 }];
  const value = reply.countries[0].operatives[0].runs[0].value;
  value.assignmentOperation = true; value.assignmentName = null;
  assert.equal(rosterAt(reply, 110, false).rows[0].assignmentReadHour, null);
  value.assignmentIcon = 0;
  assert.equal(rosterAt(reply, 110, false).rows[0].assignmentReadHour, 97);
  value.assignmentOperation = false;
  assert.equal(rosterAt(reply, 110, false).rows[0].assignmentReadHour, null);
});

test('reply artwork and reasons resolve index zero and every run reference is bounded', () => {
  const reply = fixture();
  const value = reply.countries[0].operatives[0].runs[0].value;
  value.portrait = 0;
  value.assignmentIcon = 0;
  value.nameMissing = 0;
  validateOperativesRosterReply(reply);
  const tip = operativeTooltip(reply, rosterAt(reply, 110, false).rows[0], 110, tag => tag);
  assert.equal(tip.icon, reply.artwork[0]);
  assert.equal(tip.note ?? '', '');
  for (const field of ['portrait', 'assignmentIcon', 'nameMissing', 'portraitMissing', 'experienceMissing', 'assignmentMissing', 'assignmentIconMissing']) {
    for (const index of [-1, 0.5, 2 ** 32, Infinity, NaN, '0', undefined]) {
      const malformed = fixture();
      Object.assign(malformed.countries[0].operatives[0].runs[0].value, { [field]: index });
      assert.throws(() => validateOperativesRosterReply(malformed), `${field} must reject ${String(index)}`);
    }
    const outside = fixture();
    Object.assign(outside.countries[0].operatives[0].runs[0].value, { [field]: field === 'portrait' || field === 'assignmentIcon' ? outside.artwork.length : outside.reasons.length });
    assert.throws(() => validateOperativesRosterReply(outside), `${field} must reject an absent entry`);
  }
  for (const table of ['artwork', 'reasons']) {
    const absent = fixture(); Object.assign(absent, { [table]: undefined });
    assert.throws(() => validateOperativesRosterReply(absent));
    const malformed = fixture(); Object.assign(malformed, { [table]: ['not-an-icon', 42] });
    assert.throws(() => validateOperativesRosterReply(malformed));
  }
});

test('skill admits the native i32 range independently of calendar hours', () => {
  for (const skill of [-(2 ** 31), -1, 0, 2 ** 31 - 1, null]) {
    const reply = fixture(); reply.countries[0].operatives[0].runs[0].value.skill = skill;
    validateOperativesRosterReply(reply);
    assert.equal(rosterAt(reply, 110, false).rows[0].run.value.skill, skill);
  }
  for (const skill of [-(2 ** 31) - 1, 2 ** 31, 1.5, NaN, Infinity]) {
    const reply = fixture(); reply.countries[0].operatives[0].runs[0].value.skill = skill;
    assert.throws(() => validateOperativesRosterReply(reply));
  }
});

test('captured assignment keeps the native duration string and progress without interpolation', () => {
  const reply = fixture();
  const value = reply.countries[0].operatives[0].runs[1].value;
  value.assignmentDuration = 'Content duration: 17';
  for (const progress of [-0.25, 0, 0.5, 1.5]) {
    value.assignmentProgress = progress;
    validateOperativesRosterReply(reply);
    const row = rosterAt(reply, 140, false).rows[0];
    assert.equal(row.run.value.assignmentProgress, progress);
    assert.equal(operativeTooltip(reply, row, 140, tag => tag).rows.find(row => row.label === 'Assignment duration')?.value, 'Content duration: 17');
  }
  for (const progress of [NaN, Infinity, -Infinity]) {
    value.assignmentProgress = progress;
    assert.throws(() => validateOperativesRosterReply(reply));
  }
});

test('malformed replies reject overlapping runs, bad references, invalid native dates and gap bridging', () => {
  for (const mutate of [
    (reply: OperativeRosterReply) => { reply.countries[0].operatives[0].runs[1].firstHour = 123; },
    (reply: OperativeRosterReply) => { reply.countries[0].operatives[0].runs[0].value.experienceRaw = 'not-a-number'; },
    (reply: OperativeRosterReply) => { Object.assign(reply.countries[0].operatives[0].runs[0].value, { portrait: 'guessed-portrait' }); },
    (reply: OperativeRosterReply) => { reply.countries[0].operatives[0].runs[0].value.skill = 1e20; },
    (reply: OperativeRosterReply) => { reply.countries[0].operatives[0].runs[1].value.capturedHour = 125.5; },
    (reply: OperativeRosterReply) => { reply.countries[0].operatives[0].runs[1].lastHour = 219; },
    (reply: OperativeRosterReply) => { reply.countries[0].observed[2].readHour = 124; },
    (reply: OperativeRosterReply) => { reply.countries[0].operatives[0].departedHour = 200; },
    (reply: OperativeRosterReply) => { reply.countries.push(reply.countries[0]); },
    (reply: OperativeRosterReply) => { reply.countries[0].operatives.push(reply.countries[0].operatives[0]); },
    (reply: OperativeRosterReply) => { reply.countries[0].operatives[0].runs[0].value.unnamed = true; },
    (reply: OperativeRosterReply) => { Object.assign(reply.countries[0].operatives[0].runs[0].value, { unnamed: undefined }); },
    (reply: OperativeRosterReply) => { Object.assign(reply.countries[0].operatives[0].runs[0].value, { assignmentOperation: undefined }); },
    (reply: OperativeRosterReply) => { Object.assign(reply, { operations: undefined }); },
    (reply: OperativeRosterReply) => { Object.assign(reply, { agencyAbsent: undefined }); },
    (reply: OperativeRosterReply) => { reply.agencyAbsent = [{ firstHour: 100, lastHour: 123 }, { firstHour: 123, lastHour: 147 }]; },
    (reply: OperativeRosterReply) => { reply.agencyAbsent = [{ firstHour: 100, lastHour: 99 }]; },
    (reply: OperativeRosterReply) => { reply.operations = [{ firstHour: 97, lastHour: 110, readHour: 97 }, { firstHour: 110, lastHour: 123, readHour: 110 }]; },
    (reply: OperativeRosterReply) => { reply.operations = [{ firstHour: 97, lastHour: 123, readHour: 100 }]; },
    (reply: OperativeRosterReply) => { reply.operations = [{ firstHour: 97, lastHour: 110, readHour: 97 }, { firstHour: 112, lastHour: 123, readHour: 97 }]; },
  ]) {
    const reply = fixture(); mutate(reply);
    assert.throws(() => validateOperativesRosterReply(reply));
  }
});

test('5000 synthetic countries retain every operative without a status gate', () => {
  const reply = fixture();
  reply.countries = Array.from({ length: 5000 }, (_, country) => ({ ...reply.countries[0], tag: `Mod country ${country}`,
    identities: reply.countries[0].identities.map(identity => ({ ...identity, slot: country + 1, countryId: country + 1 })) }));
  const tags = reply.countries.map(country => country.tag);
  assert.deepEqual(resolveCountries({ kind: 'tags', tags }, [], undefined, 120, definition.countries.max).tags, tags);
  assert.equal(validateOperativesRosterReply(reply).countries.length, 5000);
  const rows = rosterAt(reply, 120, false).rows;
  assert.equal(rows.length, 5000);
  assert.equal(new Set(rows.map(row => row.key)).size, 5000);
  const sorted = sortRows(rows, row => row.run.value.skill, 'descending', row => row.key);
  assert.equal(sorted.length, 5000);
  assert.equal(rosterAt(reply, 120, false, tag => tag === 'Mod country 4999').rows.length, 1);
});

test('late operative episodes remain unknown until their supplying roster read', () => {
  const make = (count: number): OperativeRosterReply => {
    const reply = fixture();
    const country = reply.countries[0];
    const entry = country.operatives[0];
    country.observed = Array.from({ length: count }, (_, index) => ({ firstHour: 100 + index * 24,
      lastHour: 123 + index * 24, readHour: 100 + index * 24 }));
    const last = country.observed.at(-1)!;
    country.identities = [{ firstHour: 100, lastHour: last.lastHour, slot: 1, countryId: 7 }];
    country.operatives = Array.from({ length: count }, (_, index) => ({ ...entry, key: `late:${index}`, id: String(index),
      joinedHour: last.firstHour, runs: [{ ...entry.runs[0], firstHour: last.firstHour, lastHour: last.lastHour }] }));
    return reply;
  };
  const reply = make(5000);
  assert.equal(validateOperativesRosterReply(reply).countries[0].operatives.length, 5000);
  const latest = reply.countries[0].observed.at(-1)!;
  assert.equal(rosterAt(reply, latest.firstHour - 1, false).rows.length, 0);
  assert.equal(rosterAt(reply, latest.firstHour, false).rows.length, 5000);
  assert.equal(rosterAt(reply, latest.lastHour + 1, false).unknown.length, 1);
});

test('settings persist all silent look choices and Live starts on', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.equal(definition.defaultTime, 'follow');
  assert.equal(options.departed, true);
  assert.equal(options.showRank, true);
  assert.equal(options.cellBars, true);
  assert.equal(options.rankMarkers, true);
  assert.equal(options.density, 'comfortable');
  assert.deepEqual(DEFAULT_SORT, { column: 'name', direction: 'ascending' });
  assert.equal(resolveOptions(OPTIONS, { departed: false }).departed, false);
});
