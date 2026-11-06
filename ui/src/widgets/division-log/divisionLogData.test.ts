import assert from 'node:assert/strict';
import test from 'node:test';
import { deepFreeze } from '../../testFrozen.ts';
import type { DivisionLogEntry, DivisionLogReply } from '../../generated/dashboard';
import { divisionRows, divisionStatus, divisionTooltip, selectedTemplates, templateToken, validateDivisionLog,
  validateDivisionManifest, validateDivisionTotals } from './divisionLogData.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { battleWindows } from '../land-battles/landBattlesData.ts';

const entry = (id = '1'): DivisionLogEntry => ({ id, division: id, readHour: 12, hour: 11, by: false, afterHour: null,
  kind: 'lost', status: 'division_destroyed', tag: 'mod/tag', countryHour: 12, name: 'Guards', nameMissing: null,
  icon: null, iconMissing: 'Not recorded', presentationHour: 10, template: '99', templateName: 'Guards template',
  templateMissing: null, templateHour: 10, province: 17, placeHour: 11, placeAtEvent: true,
  killerTag: 'other', killerHour: 12, killerDivision: '2' });
const reply = (rows = [entry()], window = 0): DivisionLogReply => ({ window, interval: { firstHour: 10, lastHour: 20 }, rows, warnings: [], contentWarnings: [] });
const rows = (at = 12, first = 10, events = 'both', templates: string[] | null = null, all = true) =>
  divisionRows(new Map([[0, reply()]]), at, first, events, templates, () => false, all);

test('division rows and objects persist between supplying reads and replacement replies are picked up', () => {
  const data = deepFreeze(reply([entry(), { ...entry('2'), hour: 18, readHour: 20 }]));
  const windows = deepFreeze(new Map([[0, data]])), matches = () => true;
  const at = (hour: number) => divisionRows(windows, hour, 10, 'both', null, matches, true);
  const first = at(12);
  assert.equal(at(19), first);
  const crossed = at(20);
  assert.equal(crossed.length, 2);
  assert.equal(crossed[1], first[0]);
  const replacement = deepFreeze(new Map([[0, { ...data, rows: data.rows.map(row => ({ ...row, name: 'Replacement' })) }]]));
  assert.equal(divisionRows(replacement, 20, 10, 'both', null, matches, true)[0].item.name, 'Replacement');
});

test('death dates do not make their later captured read visible early', () => {
  assert.equal(rows(11).length, 0);
  assert.equal(rows(12).length, 1);
  assert.equal(rows(12, 12).length, 0);
  assert.equal(rows(12, 10, 'created').length, 0);
  assert.equal(rows(12, 10, 'lost').length, 1);
});
test('country and template filters retain unknowns only in all-country logs', () => {
  assert.equal(rows(12, 10, 'both', ['other']).length, 0);
  assert.equal(rows(12, 10, 'both', []).length, 0);
  assert.equal(rows(12, 10, 'both', ['99']).length, 1);
  assert.equal(rows(12, 10, 'both', null, false).length, 0);
  const unknown = reply([{ ...entry(), tag: null }]);
  assert.equal(divisionRows(new Map([[0, unknown]]), 12, 10, 'both', null, () => true, false).length, 0);
  const reads: number[] = [];
  divisionRows(new Map([[0, reply()]]), 12, 10, 'both', null, (_tag, hour) => { reads.push(hour); return true; }, false);
  assert.deepEqual(reads, [12]);
});
test('window completion order does not change newest-first order or omit 5000 countries', () => {
  const first = reply(Array.from({ length: 5000 }, (_, i) => ({ ...entry(String(i + 1)), tag: `mod/tag-${i}`,
    hour: 10, readHour: 10, countryHour: 10, placeHour: 10, killerHour: 10 })));
  const next = reply([{ ...entry('5001'), hour: 12, readHour: 12 }], 1);
  const ordered = divisionRows(new Map([[1, next], [0, first]]), 12, 10, 'both', null, () => true, true);
  assert.equal(ordered.length, 5001);
  assert.equal(ordered[0].key, '5001');
  assert.equal(validateDivisionLog(first).rows.length, 5000);
  assert.equal(new Set(first.rows.map(row => row.tag)).size, 5000);
});
test('template handles are scoped to campaign and branch, and unavailable choices select none', () => {
  const chosen = templateToken('c0ffee', '1', '99');
  assert.deepEqual(selectedTemplates(chosen, 'c0ffee', '1'), ['99']);
  assert.deepEqual(selectedTemplates(chosen, 'beef', '1'), []);
  assert.deepEqual(selectedTemplates(chosen, 'c0ffee', '2'), []);
  assert.equal(selectedTemplates('', 'c0ffee', '1'), null);
});
test('malformed replies and future presentation dates refuse', () => {
  assert.throws(() => validateDivisionLog(reply([entry(), entry()])), /malformed/);
  assert.throws(() => validateDivisionLog(reply([{ ...entry(), presentationHour: 13 }])), /malformed/);
  assert.throws(() => validateDivisionTotals({ created: -1, lost: 0, transferred: 0, since: null, warnings: [] }), /malformed/);
  assert.throws(() => validateDivisionTotals({ created: 1, lost: 0, transferred: 0, since: 1.5, warnings: [] }), /malformed/);
  assert.equal(validateDivisionTotals({ created: 5000, lost: 3, transferred: 2, since: null, warnings: [] }).created, 5000);
  assert.equal(validateDivisionTotals({ created: 5, lost: 0, transferred: 0, since: 400, warnings: [] }).since, 400);
  assert.throws(() => validateDivisionManifest({ windows: [], available: true, lossesAvailable: true,
    templates: [{ id: '99', name: 'A' }, { id: '99', name: 'B' }], contentWarnings: [] }), /malformed/);
});
test('tooltips retain native disposition, bounded dates and the supplying read', () => {
  assert.equal(divisionStatus({ ...entry(), status: 'division_disbanded' }), 'Disbanded');
  assert.equal(divisionStatus({ ...entry(), status: 'first_observed' }), 'First seen');
  assert.equal(divisionStatus({ ...entry(), status: 'volunteer_transferred' }), 'Volunteer transfer');
  const tooltip = divisionTooltip({ ...entry(), by: true, afterHour: 10, placeAtEvent: false }, { name: 'Owner' }, { name: 'Enemy' }, 'A place');
  assert.ok(tooltip.rows.some(row => row.label === 'Last seen at'));
  assert.ok(!tooltip.rows.some(row => ['Available from', 'Template read', 'Division presentation read'].includes(row.label)));
  assert.ok(tooltip.rows.some(row => row.label === 'When' && row.value?.startsWith('By ')));
});
test('country tooltip flags retain pending and named failure state without replacing division artwork', () => {
  const country = { name: 'Owner', flagPending: true };
  const killer = { name: 'Enemy', flagPending: false, flagReason: 'The flag request failed.' };
  const tooltip = divisionTooltip({ ...entry(), icon: 'http://artwork.localhost/dddddddddddddddddddddddddddddddd.png' }, country, killer, 'A place');
  assert.equal(tooltip.icon, 'http://artwork.localhost/dddddddddddddddddddddddddddddddd.png');
  assert.deepEqual(tooltip.rows.find(row => row.label === 'Country')?.countryFlag,
    { flag: undefined, flagReason: undefined, flagPending: true });
  assert.deepEqual(tooltip.rows.find(row => row.label === 'Killer')?.countryFlag,
    { flag: undefined, flagReason: 'The flag request failed.', flagPending: false });
  const unknown = divisionTooltip({ ...entry(), tag: null, killerTag: null }, country, killer, 'A place');
  assert.equal(unknown.rows.find(row => row.label === 'Country')?.countryFlag, undefined);
  assert.equal(unknown.rows.find(row => row.label === 'Killer')?.countryFlag, undefined);
});

test('shared log defaults and bounded history windows match style five', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.equal(options.density, 'comfortable');
  assert.equal(options.months, false);
  assert.equal(options.chips, true);
  assert.equal(options.rowAnimation, true);
  assert.equal(options.events, 'both');
  assert.equal(options.totals, false);
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const windows = Array.from({ length: 5000 }, (_, i) => ({ firstHour: i * 10, lastHour: i * 10 + 9 }));
  assert.deepEqual(battleWindows({ windows }, 1000, true), [99, 100, 101]);
});
