import assert from 'node:assert/strict';
import test from 'node:test';
import type { LandBattlesManifestReply, CountryGroupReply } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import type { LandBattle, LandBattlesWindow } from './landBattlesData.ts';
import { battleWarnings, battleWindowIndex, battleWindows, rankedLandBattleRows, landBattleRows, landBattlePlaceRequests, namedLandBattles,
  shownCensusHour, validateLandBattlesReply, validateLandBattlesManifest, validateLandBattlesRanking } from './landBattlesData.ts';
import { validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import { casualtyRankingNote, landCasualtiesMissing, landCasualtyText, landDivisionText, landLossText } from './landBattlesData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups.ts';

const history = (id = 'episode'): LandBattle => ({ id, provinceId: 17, start: { hour: 10, bound: 'exact' }, end: { hour: 21, bound: 'exact' },
  endUnknownHour: null, counterBase: 0, priorPeak: null, losses: [], result: null,
  runs: [{ firstHour: 10, lastHour: 20, counts: [3, 2], rosterUnknown: false, countries: [['mod/tag'], ['other']], countriesUnknown: [false, false], duration: 1 }] });
const reply = (rows = [history()], window = 0): LandBattlesWindow => ({ window, interval: { firstHour: 10, lastHour: 21 },
  rows, observed: [[10, 21]], warnings: [], contentWarnings: [] });
/** The product's compact form: interned labels and warning reasons. */
const wire = (rows: unknown[] = [history()], labels: string[] = []) => ({ ...reply([]), labels, rows, reasons: [], warnings: [] });
const read = (h = history(), at = 20, all = true, matches = (_tag: string, _hour: number) => true) => landBattleRows(new Map([[0, reply([h])]]), at, matches, all);
const countryGroups = (): CountryGroupReply => ({ branchId: '1', warnings: [], identityChanges: [], factions: [], wars: [],
  warStarts: [], capitulations: [], humans: [{ tag: 'other', firstHour: 10, lastHour: 40 }],
  unobserved: { humans: [{ firstHour: 19, lastHour: 20 }], factions: [], wars: [] },
  countryIdentities: [
    { tag: 'mod/tag', identities: [{ firstHour: 10, lastHour: 40, slot: 1, countryId: 11 }] },
    { tag: 'other', identities: [{ firstHour: 10, lastHour: 40, slot: 2, countryId: 22 }] },
    { tag: 'former/other', identities: [{ firstHour: 1, lastHour: 9, slot: 2, countryId: 22 }] },
  ],
});

test('recorded final winners wait for their supplying hour and terminal-only rosters stay unknown', () => {
  const h = history();
  h.result = { hour: 21, duration: 25, winner: { side: 1 }, countries: [['late attacker'], ['late defender']], countriesUnknown: [false, false] };
  assert.deepEqual(read(h, 20)[0].winner, { unknown: 'Result not recorded' });
  assert.deepEqual(read(h, 21)[0].winner, { side: 1 });
  assert.equal(read(h, 21)[0].sides[0].countries[0].tag, 'late attacker');
  assert.equal(read(h, 21)[0].sides[0].countries[0].hour, 21);
  assert.equal(read(h, 21)[0].length.value, 25);
  const transient = { ...h, start: { hour: 21, bound: 'by' as const },
    runs: [{ ...h.runs[0], firstHour: 21, lastHour: 21, counts: [0, 0] as [number, number], rosterUnknown: true }] };
  const row = read(transient, 21)[0];
  assert.equal(row.peakUnknown, true);
  assert.equal(row.sides[0].counts[0].value, null);
  assert.equal(validateLandBattlesReply(wire([transient])).rows.length, 1);
});

test('division text pluralises and compact summaries retain exact values without repeated unit labels', () => {
  assert.equal(landDivisionText({ label: 'Divisions (incl. reserves)', value: 1 }, false), '1 division');
  assert.equal(landDivisionText({ label: 'Divisions (incl. reserves)', value: 2 }, false), '2 divisions');
  assert.equal(landDivisionText({ label: 'Divisions (incl. reserves)', value: 0 }, false), '0 divisions');
  assert.equal(landDivisionText({ label: 'Divisions (incl. reserves)', value: 5000 }, true), '5,000');
  assert.equal(landDivisionText({ label: 'Divisions (incl. reserves)', value: null, unknown: 'Not recorded' }, true), '—');
  assert.equal(landCasualtiesMissing(read()), true);
  assert.equal(landCasualtiesMissing([]), false);
  assert.equal(landCasualtiesMissing([{ ...read()[0], casualties: { label: 'Casualties', value: 0 } }]), false);
});

test('window replies keep all entries and actual counters; malformed runs refuse', () => {
  assert.equal(validateLandBattlesReply(wire()).rows.length, 1);
  const rows = Array.from({ length: 5000 }, (_, i) => history(String(i)));
  assert.equal(validateLandBattlesReply(wire(rows)).rows.length, 5000);
  assert.throws(() => validateLandBattlesReply(wire([history(), history()])), /malformed/);
  assert.throws(() => validateLandBattlesReply(wire([{ ...history(), runs: [{ ...history().runs[0], duration: 2147483648 }] }])), /malformed/);
});

test('exact endings wait for the absent read; tail and gaps retain unknown endings', () => {
  assert.equal(read()[0].end, null);
  assert.deepEqual(read(history(), 21)[0].end, { hour: 21, bound: 'exact' });
  const unknown = { ...history(), end: { unknown: 'Ending not recorded' }, endUnknownHour: 21 };
  assert.equal(read(unknown, 20)[0].end, null);
  assert.deepEqual(read(unknown, 21)[0].end, { unknown: 'Ending not recorded' });
  assert.deepEqual(read({ ...history(), end: null }, 40)[0].end, { unknown: 'Ending not recorded' });
  assert.deepEqual(read()[0].winner, { unknown: 'Result not recorded' });
  assert.equal(read()[0].sides[0].counts[0].label, 'Divisions (incl. reserves)');
  assert.equal(landBattleRows(new Map([[0, reply()]]), 21, () => true, true, 21).length, 1);
});

test('peaks retain the earliest supplying hour, lengths use native rounds without gap carry', () => {
  const h = history();
  h.priorPeak = { value: 10, hour: 8 };
  h.start = { hour: 8, bound: 'by' };
  const row = read(h)[0];
  assert.deepEqual(row.peak, { value: 10, hour: 8 });
  assert.equal(row.length.value, 12);
  const split = { ...h, start: { hour: 10, bound: 'by' as const }, priorPeak: null, counterBase: 50,
    runs: [{ ...h.runs[0], duration: 50 }] };
  assert.equal(read(split)[0].length.value, 10);
  assert.deepEqual(row.casualties, { label: 'Casualties', value: null, unknown: 'Not in this recording' });
});

test('network order and prefetched future windows cannot leak later reads or peaks', () => {
  const early = reply();
  const later = reply([{ ...history(), priorPeak: { value: 100, hour: 25 }, end: null,
    runs: [{ firstHour: 30, lastHour: 31, counts: [50, 50], rosterUnknown: false, countries: [['future'], []], countriesUnknown: [false, false], duration: 21 }] }], 1);
  later.interval = { firstHour: 30, lastHour: 40 };
  const rows = landBattleRows(new Map([[1, later], [0, early]]), 20, () => true, true);
  assert.equal(rows[0].peak.value, 5);
  assert.equal(rows[0].place.hour, 10);
  assert.equal(landBattleRows(new Map([[1, later], [0, early]]), 31, () => true, true)[0].peak.value, 100);
  const manifest = { available: true, unavailable: null, contentWarnings: [], windows: [{ firstHour: 10, lastHour: 21 }, { firstHour: 30, lastHour: 40 }] };
  assert.deepEqual(battleWindows(manifest, 20, true), [0, 1]);
  assert.deepEqual(battleWindows(manifest, 35, false), [0, 1]);
});

test('client countryMatch filters either side before ranking, including group membership gaps', () => {
  const groups = countryGroups();
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
  const match = countryMatch(selection, [], groups, 40);
  assert.equal(read(history(), 20, false, match.matches).length, 0);
  assert.equal(read(history(), 18, false, match.matches).length, 1);
  const chosen = countryMatch({ kind: 'tags', tags: ['other'] }, [], groups, 40);
  assert.equal(read(history(), 20, false, chosen.matches).length, 1);
  assert.equal(chosen.matches('former/other', 5), true);
  assert.equal(chosen.matches('former/other', 10), false);
  assert.equal(chosen.matches('mod/tag', 18), false);
  assert.equal(read(history(), 20, false, countryMatch({ kind: 'tags', tags: ['departed'] }, [], groups, 40).matches).length, 0);
});

test('pending native country catalogues refuse manual rows and serialize an empty ranking predicate', () => {
  const selection: CountrySelection = { kind: 'tags', tags: ['other'] };
  const pending = countryMatch(selection, [], undefined, 40);
  assert.equal(pending.ready, false);
  assert.equal(pending.reason, null);
  assert.equal(read(history(), 20, false, pending.matches).length, 0);
  assert.deepEqual(countryMatchFilter(selection, [], undefined, 40), { tags: null, membership: [], unobserved: [] });
});

test('diagnostics stop at the selected hour; shown-read status dates the census, not a battle', () => {
  const data = validateLandBattlesReply({ ...wire(), reasons: ['shown', 'future'], warnings: [[12, 0, 1], [21, 1, 2]] });
  assert.deepEqual(data.warnings, [{ hour: 12, reason: 'shown', count: 1 }, { hour: 21, reason: 'future', count: 2 }]);
  const windows = new Map([[0, data]]);
  assert.equal(shownCensusHour(windows, 21), 21);
  assert.deepEqual(battleWarnings(windows, 10, 20), ['Hour 12: shown (1)']);
  assert.throws(() => validateLandBattlesReply({ ...wire(), reasons: ['shown'], warnings: [[12, 1, 1]] }), /malformed/);
});

test('all land reply validators require plain content diagnostics, separate from dated battle warnings', () => {
  const manifest = { available: true, unavailable: null, windows: [], contentWarnings: ['Content file unavailable'] };
  const data = { ...wire(), contentWarnings: manifest.contentWarnings };
  const ranking = { labels: [], rows: [], censusHour: null, effectiveMeasure: 'Peak size', warnings: [], contentWarnings: manifest.contentWarnings };
  assert.deepEqual(validateLandBattlesManifest(manifest).contentWarnings, manifest.contentWarnings);
  assert.deepEqual(validateLandBattlesReply(data).contentWarnings, manifest.contentWarnings);
  assert.deepEqual(validateLandBattlesRanking(ranking).contentWarnings, manifest.contentWarnings);
  for (const contentWarnings of [undefined, null, 'warning', [1], [{ hour: 12, reason: 'dated', count: 1 }]]) {
    assert.throws(() => validateLandBattlesManifest({ ...manifest, contentWarnings }), /malformed/);
    assert.throws(() => validateLandBattlesReply({ ...data, contentWarnings }), /malformed/);
    assert.throws(() => validateLandBattlesRanking({ ...ranking, contentWarnings }), /malformed/);
  }
});

test('loaded windows report each content diagnostic once while battle warnings stay dated', () => {
  const early = { ...reply(), contentWarnings: ['Shared content failure', 'First window failure'],
    warnings: [{ hour: 12, reason: 'shown', count: 1 }] };
  const later = { ...reply([], 1), contentWarnings: ['Shared content failure', 'Later window failure'],
    warnings: [{ hour: 21, reason: 'future', count: 1 }] };
  assert.deepEqual(battleWarnings(new Map([[0, early], [1, later]]), 10, 20), [
    'Shared content failure', 'First window failure', 'Hour 12: shown (1)', 'Later window failure',
  ]);
});

test('window and casualty searches work without ES2023 array methods', () => {
  const windows = [{ firstHour: 10, lastHour: 21 }, { firstHour: 30, lastHour: 40 }];
  Object.defineProperty(windows, 'findLastIndex', { value: undefined });
  const manifest = { available: true, unavailable: null, windows, contentWarnings: [] };
  assert.equal(battleWindowIndex([], 10), -1);
  assert.equal(battleWindowIndex(windows, 9), -1);
  assert.equal(battleWindowIndex(windows, 10), 0);
  assert.equal(battleWindowIndex(windows, 29), 0);
  assert.equal(battleWindowIndex(windows, 40), 1);
  assert.deepEqual(battleWindows(manifest, 20, true), [0, 1]);
  const h = history();
  h.losses = [{ hour: 10, terminal: false, sides: [[{ label: 'Manpower lost', value: 3 }], [{ label: 'Manpower lost', value: 5 }]] },
    { hour: 21, terminal: true, sides: [[{ label: 'Manpower lost', value: 7 }], [{ label: 'Manpower lost', value: 9 }]] }];
  Object.defineProperty(h.losses, 'findLast', { value: undefined });
  assert.equal(read(h, 20)[0].casualties.value, 8);
  assert.equal(read(h, 21)[0].casualties.value, 16);
});

test('only drawn province/hour pairs ask for names and names never backfill earlier reads', () => {
  const visible = [read()[0], { ...read()[0], id: 'earlier', place: { name: null, hour: 19 } }];
  assert.deepEqual(landBattlePlaceRequests([...visible, visible[0]]), [{ provinceId: 17, hour: 10 }, { provinceId: 17, hour: 19 }]);
  const places = validatePlaceNamesReply({ places: [{ provinceId: 17, hour: 10, name: 'Mod town, Mod state', missing: null }], states: [], warnings: [] });
  assert.equal(namedLandBattles(visible, places)[0].place.name, 'Mod town, Mod state');
  assert.equal(namedLandBattles(visible, places)[1].place.name, null);
  assert.throws(() => validatePlaceNamesReply({ ...places, places: [places.places[0], places.places[0]] }), /malformed/);
});

test('place names require state/hour replies, retain renames and accept every requested state', () => {
  const states = [{ stateId: 17, hour: 10, name: 'First state name', missing: null },
    { stateId: 17, hour: 19, name: 'Renamed state', missing: null },
    { stateId: 18, hour: 10, name: null, missing: 'Not in this recording: state 18 at this hour' }];
  const data = { places: [{ provinceId: 17, hour: 10, name: 'Town', missing: null }], states, warnings: [] };
  assert.deepEqual(validatePlaceNamesReply(data), data);
  const many = { places: [], states: Array.from({ length: 5000 }, (_, stateId) => ({ ...states[0], stateId })), warnings: [] };
  assert.deepEqual(validatePlaceNamesReply(many), many);
  for (const invalid of [undefined, null, {}, [states[0], states[0]],
    [{ ...states[0], stateId: -1 }], [{ ...states[0], stateId: 4294967296 }], [{ ...states[0], stateId: 1.5 }],
    [{ ...states[0], hour: 2147483648 }], [{ ...states[0], hour: 1.5 }],
    [{ ...states[0], name: '' }], [{ ...states[0], name: 1 }], [{ ...states[0], missing: undefined }],
    [{ ...states[2], missing: null }], [{ ...states[2], missing: '' }]]) {
    assert.throws(() => validatePlaceNamesReply({ ...data, states: invalid }), /malformed/);
  }
});

test('definition follows the cap ruling and offers casualty default plus uncapped user top N', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  assert.equal(definition.availability.product, 'land-battles-manifest');
  assert.equal(resolveOptions(OPTIONS, {}).measure, 'Casualties');
  assert.equal(resolveOptions(OPTIONS, {}).view, 'log');
  assert.equal(resolveOptions(OPTIONS, {}).topN, 30);
  for (const topN of [5000, Number.MAX_SAFE_INTEGER, 1e100]) assert.equal(resolveOptions(OPTIONS, { topN }).topN, topN);
  for (const measure of ['Casualties', 'Peak size', 'Length']) assert.equal(resolveOptions(OPTIONS, { measure }).measure, measure);
  const top = OPTIONS.find(option => option.key === 'topN');
  assert.ok(top?.kind === 'number');
  assert.equal(top.max, Infinity);
});

test('at the last complete census open battles remain ongoing; hidden subsequent reads are unknown', () => {
  const h = { ...history(), end: null };
  assert.equal(read(h, 20)[0].end, null);
  assert.deepEqual(read(h, 21)[0].end, { unknown: 'Ending not recorded' });
  assert.equal(read(h, 19)[0].end, null);
  assert.equal(read(h, 20)[0].place.hour, read(h, 19)[0].place.hour);
  assert.deepEqual(landBattlePlaceRequests(read(h, 20)), landBattlePlaceRequests(read(h, 19)));
  assert.equal(read(h, 20)[0].readHour, 20);
  assert.equal(read(h, 19)[0].sides[0].countries[0].hour, 19);
});

test('nine years wants playhead, next Live and drawn history only; release scrolled windows', () => {
  const manifest: LandBattlesManifestReply = { available: true, unavailable: null, contentWarnings: [],
    windows: Array.from({ length: 110 }, (_, i) => ({ firstHour: i * 720, lastHour: i * 720 + 719 })) };
  assert.equal(validateLandBattlesManifest(manifest).windows.length, 110);
  assert.deepEqual(battleWindows(manifest, 78_000, false), [107, 108]);
  assert.deepEqual(battleWindows(manifest, 78_000, true), [107, 108, 109]);
  assert.deepEqual(battleWindows(manifest, 78_000, true, [106]), [105, 106, 108, 109]);
  assert.deepEqual(battleWindows(manifest, 78_000, true, [104]), [103, 104, 108, 109]);
  assert.deepEqual(battleWindows(manifest, 78_000, false, [], 78_000), [108]);
});

test('unknown participants pass country selection and carry an explicit side mark', () => {
  const h = history(); h.runs[0].countriesUnknown = [true, false]; h.runs[0].countries[0] = [];
  const rows = read(h, 20, false, () => false);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].sides[0].countriesUnknown, 'Countries not recorded');
  assert.equal(rows[0].sides[1].countriesUnknown, undefined);
});

test('server ranking predicate equals countryMatch, and ranked summaries retain whole-battle measures', () => {
  const groups = countryGroups();
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
  const filter = countryMatchFilter(selection, [], groups, 40);
  assert.deepEqual(filter, { tags: null, membership: groups.humans, unobserved: groups.unobserved.humans });
  assert.deepEqual(countryMatchFilter({ kind: 'tags', tags: ['other'] }, [], groups, 40), { tags: null,
    membership: [{ tag: 'former/other', firstHour: 1, lastHour: 9 }, { tag: 'other', firstHour: 10, lastHour: 40 }], unobserved: [] });
  const ranked = { labels: [], rows: [history()], censusHour: 20, effectiveMeasure: 'Peak size', warnings: [], contentWarnings: [] };
  const ranking = validateLandBattlesRanking(ranked);
  assert.equal(rankedLandBattleRows(ranking, 20)[0].peak.value, 5);
  assert.equal(rankedLandBattleRows(ranking, 20)[0].length.value, 11);
  assert.throws(() => validateLandBattlesRanking({ ...ranked, effectiveMeasure: 'Guess' }), /malformed/);
});

test('casualties populate side losses and preserve variant values without mixing equipment into manpower', () => {
  const h = history();
  h.losses = [{ hour: 10, terminal: false, sides: [
    [{ label: 'Manpower lost', value: 3 }, { label: 'Equipment lost', value: 1.125 }, { label: 'Equipment lost: mod variant', value: 1.125 }],
    [{ label: 'Manpower lost', value: 5 }, { label: 'Equipment lost', value: 2 }],
  ] }];
  const row = read(h)[0];
  assert.equal(row.casualties.value, 8);
  assert.deepEqual(row.sides[0].losses, h.losses[0].sides[0]);
  assert.equal(landCasualtyText(row.casualties, row), '8 so far');
  assert.equal(landCasualtyText(row.casualties, row, true), `8 so far, as of ${formatDateTime(10)}`);
  assert.equal(landLossText(row.sides[0].losses[1], row, 0, true), `1.125 so far, as of ${formatDateTime(10)}`);
});

test('terminal values are shown only at their supplying hour, and absence cannot finalize losses', () => {
  const h = history();
  h.losses = [{ hour: 10, terminal: false, sides: [[{ label: 'Manpower lost', value: 3 }], [{ label: 'Manpower lost', value: 5 }]] },
    { hour: 21, terminal: true, sides: [[{ label: 'Manpower lost', value: 7 }], [{ label: 'Manpower lost', value: 9 }]] }];
  const running = read(h, 20)[0], ended = read(h, 21)[0];
  assert.equal(running.casualties.value, 8);
  assert.equal(ended.casualties.value, 16);
  assert.equal(landCasualtyText(ended.casualties, ended), '16');
  assert.equal(landCasualtyText(ended.casualties, ended, true), `16 terminal, as of ${formatDateTime(21)}`);
  h.losses.pop();
  const missingTerminal = read(h, 21)[0];
  assert.equal(landCasualtyText(missingTerminal.casualties, missingTerminal), '8 so far');
  assert.equal(missingTerminal.lossRead?.terminal, false);
});

test('compact loss reads rebuild the full reads and loss text, with fixed5 fractions and wide signed corrections', () => {
  const labels = ['Equipment lost · mod variant (variant 7)', 'Equipment lost · Name not recorded (variant 8)', 'Equipment lost · wide (variant 9)'];
  const h = { ...history(), losses: [
    { hour: 10, terminal: false, sides: [{ manpower: 3, variants: [[0, '-100001'], [1, '250000']] }, { manpower: 'Not in this recording', unknown: 'Not in this recording' }] },
    { hour: 11, terminal: false, sides: [{ manpower: 4, changed: [[1, '300000']] }, { manpower: 6, variants: [[2, '-9223372036854775807']] }] },
    { hour: 12, terminal: false, sides: 'Hourly losses not recorded' },
    { hour: 13, terminal: true, sides: [{ manpower: 9, variants: [[1, '300000'], [0, '-100001']] }, { manpower: 7, variants: [] }] },
  ] };
  const missing = (reason: string) => [{ label: 'Manpower lost', value: null, unknown: reason }, { label: 'Equipment lost', value: null, unknown: reason }];
  const wide = 'Exact amount: -92233720368547.75807';
  const losses = validateLandBattlesReply(wire([h], labels)).rows[0].losses;
  assert.deepEqual(losses, [
    { hour: 10, terminal: false, equipmentRaw: [['149999', '-100001', '250000'], []], sides: [
      [{ label: 'Manpower lost', value: 3 }, { label: 'Equipment lost', value: 1.49999 }, { label: labels[0], value: -1.00001 }, { label: labels[1], value: 2.5 }],
      missing('Not in this recording')] },
    { hour: 11, terminal: false, equipmentRaw: [['199999', '-100001', '300000'], ['-9223372036854775807', '-9223372036854775807']], sides: [
      [{ label: 'Manpower lost', value: 4 }, { label: 'Equipment lost', value: 1.99999 }, { label: labels[0], value: -1.00001 }, { label: labels[1], value: 3 }],
      [{ label: 'Manpower lost', value: 6 }, { label: 'Equipment lost', value: null, unknown: wide }, { label: labels[2], value: null, unknown: wide }]] },
    { hour: 12, terminal: false, sides: [missing('Hourly losses not recorded'), missing('Hourly losses not recorded')] },
    { hour: 13, terminal: true, equipmentRaw: [['199999', '300000', '-100001'], ['0']], sides: [
      [{ label: 'Manpower lost', value: 9 }, { label: 'Equipment lost', value: 1.99999 }, { label: labels[1], value: 3 }, { label: labels[0], value: -1.00001 }],
      [{ label: 'Manpower lost', value: 7 }, { label: 'Equipment lost', value: 0 }]] },
  ]);
  const row = read({ ...h, losses }, 11)[0];
  assert.equal(row.casualties.value, 10);
  assert.equal(landLossText(row.sides[0].losses[2], row, 0, false), '−1.00001 so far');
  assert.equal(landLossText(row.sides[1].losses[1], row, 1, true), `−92,233,720,368,547.75807 so far, as of ${formatDateTime(11)}`);
  assert.equal(landLossText(row.sides[1].losses[2], row, 1, false), '−92,233,720,368,547.75807 so far');
});

test('missing casualties stay unknown, exact zero is retained, and ranking explains so-far or fallback', () => {
  const h = history();
  assert.deepEqual(read(h)[0].casualties, { label: 'Casualties', value: null, unknown: 'Not in this recording' });
  h.losses = [{ hour: 10, terminal: false, sides: [[{ label: 'Manpower lost', value: 0 }], [{ label: 'Manpower lost', value: null, unknown: 'Loss read unavailable' }]] }];
  assert.deepEqual(read(h)[0].casualties, { label: 'Casualties', value: null, unknown: 'Loss read unavailable' });
  h.losses[0].sides[1] = [{ label: 'Manpower lost', value: 0 }];
  assert.equal(landCasualtyText(read(h)[0].casualties, read(h)[0]), '0 so far');
  const zero = { manpower: 0, unknown: 'Not in this recording' };
  const ranking = validateLandBattlesRanking({ labels: [], rows: [{ ...h, losses: [{ hour: 10, terminal: false, sides: [zero, zero] }] }],
    censusHour: 20, effectiveMeasure: 'Casualties', warnings: [], contentWarnings: [] });
  assert.equal(rankedLandBattleRows(ranking, 20)[0].casualties.value, 0);
  assert.match(casualtyRankingNote(ranking)!, /Running battles use losses so far/);
  assert.equal(casualtyRankingNote({ ...ranking, effectiveMeasure: 'Peak size' }), 'Casualties: Not in this recording. Ranked by peak size.');
});

test('loss read validation refuses malformed sides, ambiguous labels, orphan changes and changes after terminal', () => {
  const side = { manpower: 3, unknown: 'Not in this recording' };
  const at = (hour: number, sides: unknown, terminal = false) => ({ hour, terminal, sides });
  const losses = (...reads: unknown[]) => wire([{ ...history(), losses: reads }], ['Equipment lost · mod variant (variant 7)']);
  assert.equal(validateLandBattlesReply(losses(at(10, [side, side], true))).rows.length, 1);
  for (const reads of [
    [at(10, [side, side], true), at(11, [side, side])],
    [at(10, [{ manpower: 3, variants: [[0, '1'], [0, '2']] }, side])],
    [at(10, [{ manpower: 3, variants: [[1, '1']] }, side])],
    [at(10, [{ manpower: 3, variants: [[0, '1e5']] }, side])],
    [at(10, [{ manpower: 3, changed: [] }, side])],
    [at(10, [{ manpower: 3, variants: [[0, '1']] }, side]), at(11, 'Hourly losses not recorded'), at(12, [{ manpower: 3, changed: [] }, side])],
    [at(10, [{ manpower: NaN, unknown: 'Not in this recording' }, side])],
    [at(10, [side])],
  ]) assert.throws(() => validateLandBattlesReply(losses(...reads)), /malformed/);
});

