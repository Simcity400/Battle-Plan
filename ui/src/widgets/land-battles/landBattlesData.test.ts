import assert from 'node:assert/strict';
import test from 'node:test';
import type { LandBattleHistory, LandBattlesReply, LandBattlesManifestReply, CountryGroupReply } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import { battleWarnings, battleWindowIndex, battleWindows, rankedLandBattleRows, landBattleRows, landBattlePlaceRequests, namedLandBattles,
  shownCensusHour, validatePlaceNamesReply, validateLandBattlesReply, validateLandBattlesManifest, validateLandBattlesRanking } from './landBattlesData.ts';
import { casualtyRankingNote, landCasualtiesMissing, landCasualtyText, landDivisionText, landLossText } from './landBattlesData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups.ts';

const history = (id = 'episode'): LandBattleHistory => ({ id, provinceId: 17, start: { hour: 10, bound: 'exact' }, end: { hour: 21, bound: 'exact' },
  endUnknownHour: null, counterBase: 0, priorPeak: null, losses: [],
  runs: [{ firstHour: 10, lastHour: 20, counts: [3, 2], countries: [['mod/tag'], ['other']], countriesUnknown: [false, false], duration: 1 }] });
const reply = (rows = [history()], window = 0): LandBattlesReply => ({ window, interval: { firstHour: 10, lastHour: 21 },
  rows, observed: [[10, 21]], warnings: [], contentWarnings: [] });
const read = (h = history(), at = 20, all = true, matches = (_tag: string, _hour: number) => true) => landBattleRows(new Map([[0, reply([h])]]), at, matches, all);

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
  assert.equal(validateLandBattlesReply(reply()).rows.length, 1);
  const rows = Array.from({ length: 5000 }, (_, i) => history(String(i)));
  assert.equal(validateLandBattlesReply(reply(rows)).rows.length, 5000);
  assert.throws(() => validateLandBattlesReply(reply([history(), history()])), /malformed/);
  assert.throws(() => validateLandBattlesReply(reply([{ ...history(), runs: [{ ...history().runs[0], duration: 2147483648 }] }])), /malformed/);
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
    runs: [{ firstHour: 30, lastHour: 31, counts: [50, 50], countries: [['future'], []], countriesUnknown: [false, false], duration: 21 }] }], 1);
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
  const groups = { humans: [{ tag: 'other', firstHour: 10, lastHour: 40 }], factions: [], wars: [],
    unobserved: { humans: [{ firstHour: 19, lastHour: 20 }], factions: [], wars: [] } } as unknown as CountryGroupReply;
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
  const match = countryMatch(selection, [], groups, 40);
  assert.equal(read(history(), 20, false, match.matches).length, 0);
  assert.equal(read(history(), 18, false, match.matches).length, 1);
  const chosen = countryMatch({ kind: 'tags', tags: ['other'] }, [], undefined, 40);
  assert.equal(read(history(), 20, false, chosen.matches).length, 1);
  assert.equal(read(history(), 20, false, countryMatch({ kind: 'tags', tags: ['departed'] }, [], undefined, 40).matches).length, 0);

});

test('diagnostics stop at the selected hour; shown-read status dates the census, not a battle', () => {
  const data = reply(); data.warnings = [{ hour: 12, reason: 'shown', count: 1 }, { hour: 21, reason: 'future', count: 1 }];
  const windows = new Map([[0, data]]);
  assert.equal(shownCensusHour(windows, 21), 21);
  assert.deepEqual(battleWarnings(windows, 10, 20), ['Hour 12: shown (1)']);
});

test('all land reply validators require plain content diagnostics, separate from dated battle warnings', () => {
  const manifest = { available: true, unavailable: null, windows: [], contentWarnings: ['Content file unavailable'] };
  const data = { ...reply(), contentWarnings: manifest.contentWarnings };
  const ranking = { rows: [], censusHour: null, effectiveMeasure: 'Peak size', warnings: [], contentWarnings: manifest.contentWarnings };
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
  const places = validatePlaceNamesReply({ places: [{ provinceId: 17, hour: 10, name: 'Mod town, Mod state', missing: null }], warnings: [] });
  assert.equal(namedLandBattles(visible, places)[0].place.name, 'Mod town, Mod state');
  assert.equal(namedLandBattles(visible, places)[1].place.name, null);
  assert.throws(() => validatePlaceNamesReply({ ...places, places: [places.places[0], places.places[0]] }), /malformed/);
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
  const groups = { humans: [{ tag: 'other', firstHour: 10, lastHour: 40 }], factions: [], wars: [],
    unobserved: { humans: [{ firstHour: 19, lastHour: 20 }], factions: [], wars: [] } } as unknown as CountryGroupReply;
  const selection: CountrySelection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
  const filter = countryMatchFilter(selection, [], groups, 40);
  assert.deepEqual(filter, { tags: null, membership: groups.humans, unobserved: groups.unobserved.humans });
  assert.deepEqual(countryMatchFilter({ kind: 'tags', tags: ['other'] }, [], undefined, 40), { tags: ['other'], membership: null, unobserved: [] });
  const ranking = validateLandBattlesRanking({ rows: [history()], censusHour: 20, effectiveMeasure: 'Peak size', warnings: [], contentWarnings: [] });
  assert.equal(rankedLandBattleRows(ranking, 20)[0].peak.value, 5);
  assert.equal(rankedLandBattleRows(ranking, 20)[0].length.value, 11);
  assert.throws(() => validateLandBattlesRanking({ ...ranking, effectiveMeasure: 'Guess' }), /malformed/);
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

test('missing casualties stay unknown, exact zero is retained, and ranking explains so-far or fallback', () => {
  const h = history();
  assert.deepEqual(read(h)[0].casualties, { label: 'Casualties', value: null, unknown: 'Not in this recording' });
  h.losses = [{ hour: 10, terminal: false, sides: [[{ label: 'Manpower lost', value: 0 }], [{ label: 'Manpower lost', value: null, unknown: 'Loss read unavailable' }]] }];
  assert.deepEqual(read(h)[0].casualties, { label: 'Casualties', value: null, unknown: 'Loss read unavailable' });
  h.losses[0].sides[1] = [{ label: 'Manpower lost', value: 0 }];
  assert.equal(landCasualtyText(read(h)[0].casualties, read(h)[0]), '0 so far');
  const ranking = validateLandBattlesRanking({ rows: [h], censusHour: 20, effectiveMeasure: 'Casualties', warnings: [], contentWarnings: [] });
  assert.equal(rankedLandBattleRows(ranking, 20)[0].casualties.value, 0);
  assert.match(casualtyRankingNote(ranking)!, /Running battles use losses so far/);
  assert.equal(casualtyRankingNote({ ...ranking, effectiveMeasure: 'Peak size' }), 'Casualties: Not in this recording. Ranked by peak size.');
});

test('loss read validation refuses malformed sides, ambiguous labels, and changes after terminal', () => {
  const h = history();
  h.losses = [{ hour: 10, terminal: true, sides: [[{ label: 'Manpower lost', value: 3 }], []] }];
  assert.equal(validateLandBattlesReply(reply([h])).rows.length, 1);
  h.losses.push({ ...h.losses[0], hour: 11 });
  assert.throws(() => validateLandBattlesReply(reply([h])), /malformed/);
  h.losses.pop(); h.losses[0].sides[0].push({ label: 'Manpower lost', value: 4 });
  assert.throws(() => validateLandBattlesReply(reply([h])), /malformed/);
  h.losses[0].sides[0] = [{ label: 'Manpower lost', value: NaN }];
  assert.throws(() => validateLandBattlesReply(reply([h])), /malformed/);
});

