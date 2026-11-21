import assert from 'node:assert/strict';
import test from 'node:test';
import type { LogPage } from '../../charts/log/logData';
import type { LogCounts, LogReplyRow } from '../../charts/log/logSource';
import { divisionEntry, divisionFacets, divisionNamedRows, divisionPlace, divisionStatus, gapWarnings, kindTotals, selectedTemplates, shownKinds, templateToken,
  divisionPlaces, validateDivisionManifest } from './divisionLogData.ts';
import type { DivisionKind, DivisionLogEntry } from './divisionLogData';

const reply = (key: string, hour: number, kind: number, more: Record<string, LogReplyRow['cells'][string]> = {}): LogReplyRow => ({ key, hour, fields: {}, cells: {
  division: { kind: 'text', value: key }, kind: { kind: 'number', value: String(kind) }, status: { kind: 'text', value: kind === 0 ? 'first_observed' : 'division_destroyed' },
  hour: { kind: 'number', value: String(hour) }, by: { kind: 'number', value: '1' }, readHour: { kind: 'number', value: String(hour) },
  tag: { kind: 'text', value: 'GER' }, placeAtEvent: { kind: 'number', value: kind === 0 ? '1' : '0' }, ...more,
} });

test('a page row is placed at its read, dated by its own hour, and names nothing it was not given', () => {
  const row = divisionEntry(reply('7', 120, 1, { hour: { kind: 'number', value: '119' }, by: { kind: 'number', value: '0' },
    killerTag: { kind: 'text', value: 'SOV' }, nameMissing: { kind: 'text', value: 'Division names unavailable' } }));
  assert.equal(row.hour, 120);
  assert.equal(row.item.hour, 119);
  assert.equal(row.item.by, false);
  assert.equal(row.item.kind, 'lost');
  assert.equal(row.item.killerTag, 'SOV');
  assert.equal(row.item.name, null);
  assert.equal(row.item.nameMissing, 'Division names unavailable');
  assert.equal(row.item.province, null);
  assert.equal(divisionStatus(row.item), 'Destroyed');
});

test('the log is asked for the kinds shown, the chosen templates and the countries by tag', () => {
  assert.deepEqual(shownKinds('lost'), ['lost', 'transfer']);
  assert.deepEqual(divisionFacets(['created'], null, null), [{ name: 'kind', values: [0], countries: [] }]);
  assert.deepEqual(divisionFacets(['lost', 'transfer'], ['31', '40'], ['GER']), [
    { name: 'kind', values: [1, 2], countries: [] }, { name: 'template', values: [31, 40], countries: [] }, { name: 'slot', values: [], countries: ['GER'] }]);
  const token = templateToken('campaign', '1', '31');
  assert.deepEqual(selectedTemplates(token, 'campaign', '1'), ['31']);
  assert.deepEqual(selectedTemplates(token, 'campaign', '2'), []);
});

test('each kind is counted through the hour from its bins and the held rows of the bin at the hour', () => {
  const counts = (values: number[]): LogCounts => ({ firstHour: 0, binHours: 6, counts: values, total: values.reduce((a, b) => a + b, 0), before: 0, missing: [] });
  const list = counts([2, 3]);
  const kinds = new Map<DivisionKind, LogCounts>([['created', counts([1, 2])], ['lost', counts([1, 1])]]);
  const entry = (hour: number, kind: number) => divisionEntry(reply(`${hour}:${kind}`, hour, kind));
  const page: LogPage<DivisionLogEntry> = { key: '0', start: 0, end: 4, count: 5, rows: [entry(1, 0), entry(2, 1), entry(6, 0), entry(7, 1), entry(9, 0)] };
  assert.deepEqual(kindTotals(list, kinds, [page], 4, 8), new Map([['created', 2], ['lost', 2]]));
  assert.deepEqual(kindTotals(list, kinds, [page], 2, 5), new Map([['created', 1], ['lost', 1]]));
  assert.deepEqual(kindTotals(list, kinds, [page], 5, null), new Map([['created', 3], ['lost', 2]]));
  assert.equal(kindTotals(list, kinds, [{ ...page, rows: undefined }], 4, 8), null, 'a bin whose rows are not loaded is not counted as empty');
});

test('the manifest names the hours divisions were not read, within the hours shown', () => {
  const manifest = validateDivisionManifest({ available: true, lossesAvailable: false, warnings: [], templates: [{ id: '31', name: null }],
    gaps: [{ firstHour: 10, lastHour: 20, reason: 'Divisions not read' }, { firstHour: 400, lastHour: 410, reason: 'Divisions not read' }] });
  assert.equal(gapWarnings(manifest, 0, 100).length, 1);
  assert.throws(() => validateDivisionManifest({ ...manifest, templates: [{ id: 'x', name: null }] }));
});

test('a place still being read shows as loading, never as an unnamed province; a read that named nothing, or left it out, gives its reason', () => {
  const row = divisionEntry(reply('7', 120, 0, { province: { kind: 'number', value: '42' }, placeHour: { kind: 'number', value: '118' } })).item;
  const reading = { pending: true, failed: false }, answered = { pending: false, failed: false };
  assert.equal(divisionPlace(row, new Map(), reading), 'Loading place…');
  assert.equal(divisionPlace(row, new Map(), { pending: false, failed: true }), 'Place name couldn’t load');
  assert.equal(divisionPlace(row, new Map(), answered), 'Unnamed province (the names reply left it out)', 'a place the reply left out is settled, never loading for ever');
  const key = JSON.stringify([42, 118]);
  assert.equal(divisionPlace(row, new Map([[key, { provinceId: 42, hour: 118, name: 'Brest', missing: null }]]), answered), 'Brest');
  assert.equal(divisionPlace(row, new Map([[key, { provinceId: 42, hour: 118, name: null, missing: 'Province names unavailable' }]]), answered), 'Province names unavailable');
  assert.equal(divisionPlace(divisionEntry(reply('8', 120, 0)).item, new Map(), reading), 'Location not recorded');
  assert.deepEqual(divisionPlaces([divisionEntry(reply('7', 120, 0, { province: { kind: 'number', value: '42' }, placeHour: { kind: 'number', value: '118' } })),
    divisionEntry(reply('9', 121, 0, { province: { kind: 'number', value: '42' }, placeHour: { kind: 'number', value: '118' } })), divisionEntry(reply('8', 120, 0))]),
  [{ provinceId: 42, hour: 118 }], 'each place once, none for a row without one');
  const placed = (key: string, hour: number, province: number) => divisionEntry(reply(key, hour, 0, { province: { kind: 'number', value: String(province) },
    placeHour: { kind: 'number', value: String(hour) } }));
  assert.deepEqual(divisionPlaces([placed('a', 100, 1), placed('b', 105, 2), placed('c', 110, 3), placed('d', 104, 4)], 105).map(place => place.provinceId), [2, 4, 1, 3],
    'following, the rows at the top of the list lead: those through the hour of its block, newest first, then those held past it');
});

test('following, the rows named are those of the newest pages held, before the list draws them, and any drawn beyond them', () => {
  const rows = [0, 1, 2, 3, 4, 5].map(at => divisionEntry(reply(String(at), 100 + at, 0)));
  const pages: LogPage<DivisionLogEntry>[] = [{ key: '0', start: 0, end: 2, count: 3, rows: rows.slice(0, 3) }, { key: '1', start: 3, end: 5, count: 3, rows: rows.slice(3) }];
  assert.deepEqual(divisionNamedRows(true, pages, 3, []).map(row => row.key), ['3', '4', '5'], 'named with the block, nothing drawn yet');
  assert.deepEqual(divisionNamedRows(true, pages, 3, [rows[4], rows[1]]).map(row => row.key), ['3', '4', '5', '1'], 'a row scrolled to beyond them');
  assert.deepEqual(divisionNamedRows(false, pages, 3, [rows[1]]).map(row => row.key), ['1'], 'paused or browsing: the rows drawn');
});
