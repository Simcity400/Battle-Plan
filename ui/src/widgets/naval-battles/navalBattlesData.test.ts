import assert from 'node:assert/strict';
import test from 'node:test';
import { deepFreeze } from '../../testFrozen.ts';
import { navalBattleChanges, navalBattleMeasureText, navalBattleRow, navalBattleRows, navalBattleSize, navalBattleState, navalBattleWarnings, navalBattleWindows,
  navalSideText, navalWindowPlaces, rankedNavalBattleRows, shownNavalCensusHour, validateNavalBattlesManifest, validateNavalBattlesRanking, validateNavalBattlesReply,
  NAVAL_BATTLE_DEFAULT_MEASURE } from './navalBattlesData.ts';
import { battleChangeHours, battleJumpHour, battlePlaceRequests, battlePlaceText, battleStateRows, battleTooltip, namedBattles, isBattleRow } from '../battles/battleRows.ts';
import { validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import { registerWidgets, widgetUnavailable } from '../../dashboard/definitions.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import type { BoundTrack } from '../../dashboard/model';
import { definition } from './definition.ts';
import type { NavalBattleHistory, NavalBattleRun, NavalBattlesManifestReply, NavalBattlesRankingReply, NavalBattlesReply } from '../../generated/dashboard';

test('Naval supports Pin and retains its exact moment as playback moves', () => {
  registerWidgets([definition]);
  const widget = { ...createWidget('naval-battles', 'naval', 'pinned-battle'),
    time: { mode: 'pin' as const, windows: { naval: { firstOffset: 5, lastOffset: 5 } } } };
  const track = { id: 'naval', axisOffset: 1000, branch: { firstHour: 100, lastHour: 200 } } as BoundTrack;
  assert.equal(widgetUnavailable(widget), null);
  assert.deepEqual(resolveWindow(widget, track, [track], 110), { firstHour: 105, lastHour: 105 });
  assert.deepEqual(resolveWindow(widget, track, [track], 180), { firstHour: 105, lastHour: 105 });
});

function run(firstHour = 100, lastHour = 109, result = false, provinceId: number | null = 45): NavalBattleRun {
  return { firstHour, lastHour, provinceId, row: { id: result ? 'result:800' : 'combat:900:100',
    place: { name: null, hour: result ? 99 : 100, ...(result ? { namedAt: 100 } : {}) },
    start: result ? null : { hour: 100, bound: 'by' }, end: result ? { hour: 99, bound: 'exact' } : null,
    sides: ['Attackers', 'Defenders'].map((label, side) => ({ label,
      countries: [{ tag: side ? 'other mod country' : 'opaque mod country', hour: lastHour }],
      counts: [{ label: 'Ships', value: side + 1 }, { label: 'Convoys', value: 3 }],
      losses: [{ label: 'Sunk', value: side }, { label: 'Convoys sunk', value: 1 }] })),
    winner: result ? { side: 1 } : { unknown: 'Result not recorded' },
  }, peakHour: result ? null : 100, measures: [{ label: 'Ships sunk', value: 1, atLeast: !result }, result
    ? { label: 'Peak ships', value: null, unknown: 'Not recorded' }
    : { label: 'Peak ships', value: 3, atLeast: true }, result
    ? { label: 'Length', value: null, unknown: 'Not recorded' }
    : { label: 'Length', value: lastHour - 100, atLeast: true }] };
}
function history(id = 'combat:900:100', result = false): NavalBattleHistory {
  const r = run(100, 109, result); r.row.id = id;
  return { id, provinceId: 45, runs: [r] };
}
function reply(rows = [history(), history('result:800', true)], firstHour = 100, lastHour = 109): NavalBattlesReply {
  return { window: 0, interval: { firstHour, lastHour }, rows, observed: [[firstHour, lastHour]], warnings: [], contentWarnings: [] };
}
function rows(data: NavalBattlesReply, at = 109) { return navalBattleRows(new Map([[data.window, data]]), at, () => true, true); }
test('naval playback retains settled rows and preserves ongoing reads, range admission and disappearance', () => {
  const data = deepFreeze(reply()), windows = deepFreeze(new Map([[0, data]])), matches = () => true;
  const at = (hour: number, from: number | null = null) => navalBattleRows(windows, hour, matches, true, from);
  for (const hour of [99, 100, 105, 109, 110, 120, 105]) assert.deepEqual(at(hour), rows(data, hour));
  for (const from of [null, 105]) for (let hour = 98; hour <= 112; hour++) for (const id of ['combat:900:100', 'result:800']) {
    assert.deepEqual(navalBattleRow(windows, id, hour, matches, true, from), at(hour, from).find(row => row.id === id) ?? null, `${id} at ${hour}`);
  }
  assert.notEqual(at(105), at(106));
  assert.equal(at(110), at(120));
  assert.equal(at(104, 105).length, 0);
  assert.equal(at(105, 105).length, 1);
  const ranking = deepFreeze({ rows: data.rows, censusHour: 120, effectiveMeasure: 'Length', warnings: [], contentWarnings: [] });
  assert.equal(rankedNavalBattleRows(ranking, 110), rankedNavalBattleRows(ranking, 120));
});
function rejects(edit: (v: any) => void) { const data = reply(); edit(data); assert.throws(() => validateNavalBattlesReply(data)); }

test('window contract requires overlapping runs and all three wire measures; arbitrary labels remain valid', () => {
  assert.equal(validateNavalBattlesReply(reply()).rows.length, 2);
  for (const edit of [
    (v: any) => { v.rows.push(v.rows[0]); },
    (v: any) => { v.window = null; },
    (v: any) => { v.rows[0].runs[0].firstHour = 99; },
    (v: any) => { v.rows[0].runs[0].lastHour = 110; },
    (v: any) => { v.rows[0].runs[0].row.sides[0].countries[0].hour = 100; },
    (v: any) => { v.rows[0].runs[0].measures.pop(); },
    (v: any) => { v.rows[0].runs[0].measures[1].label = 'Ships sunk'; },
    (v: any) => { v.rows[0].runs[0].measures[1].label = 'Unsent measure'; },
    (v: any) => { v.rows[0].runs[0].peakHour = 101; },
    (v: any) => { v.rows[0].runs[0].peakHour = null; },
    (v: any) => { delete v.rows[0].runs[0].peakHour; },
    (v: any) => { v.rows[0].runs[0].row.sides.pop(); },
    (v: any) => { v.rows[0].runs[0].row.end = { hour: 101, bound: 'exact' }; },
    (v: any) => { v.rows[0].runs[0].row.place.hour = 101; },
    (v: any) => { v.rows[1].runs[0].row.place.namedAt = 98; },
    (v: any) => { v.rows[0].runs.push(v.rows[0].runs[0]); },
    (v: any) => { v.warnings = [{ hour: 99, reason: 'bad read', count: 1 }]; },
    (v: any) => { v.observed = [[100, 110]]; },
  ]) rejects(edit);
  const data = reply(); data.rows[0].runs[0].row.sides[0].label = 'Modded side';
  data.rows[0].runs[0].row.sides[0].counts.push({ label: 'Mod statistic', value: 0.125 });
  assert.doesNotThrow(() => validateNavalBattlesReply(data));
});

test('shared validators reject ambiguous winners, malformed values, units and duplicate labels', () => {
  for (const edit of [
    (v: any) => { v.rows[0].runs[0].row.winner = { side: 2 }; },
    (v: any) => { v.rows[0].runs[0].row.winner = { side: 0, unknown: 'Guess' }; },
    (v: any) => { v.rows[0].runs[0].row.end = { hour: 99, unknown: 'Unknown' }; },
    (v: any) => { v.rows[0].runs[0].row.sides[0].units = []; },
    (v: any) => { v.rows[0].runs[0].row.sides[0].countriesUnknown = 42; },
    (v: any) => { v.rows[0].runs[0].row.sides[0].counts.push({ label: 'Ships', value: 0 }); },
    (v: any) => { v.rows[0].runs[0].measures[0] = { label: 'Ships sunk', value: null }; },
    (v: any) => { v.rows[0].runs[0].measures[0].value = Infinity; },
    (v: any) => { v.rows[0].runs[0].row.id = 'other history'; },
  ]) rejects(edit);
});

test('manifest windows load playhead, drawn neighbours and Live prefetch, never the entire range', () => {
  const manifest: NavalBattlesManifestReply = { available: true, unavailable: null, contentWarnings: [],
    windows: Array.from({ length: 110 }, (_, i) => ({ firstHour: i * 720, lastHour: i * 720 + 719 })) };
  assert.equal(validateNavalBattlesManifest(manifest), manifest);
  assert.deepEqual(navalBattleWindows(manifest, 78000, false), [107, 108]);
  assert.deepEqual(navalBattleWindows(manifest, 78000, true), [107, 108, 109]);
  assert.deepEqual(navalBattleWindows(manifest, 78000, true, [104]), [103, 104, 108, 109]);
  assert.deepEqual(navalBattleWindows(manifest, 78000, false, [], 78000), [108]);
  assert.deepEqual(navalBattleWindows(undefined, 78000, true), []);
  assert.equal(validateNavalBattlesManifest({ windows: [], available: false, unavailable: 'Not in this recording', contentWarnings: [] }).available, false);
  assert.throws(() => validateNavalBattlesManifest({ ...manifest, windows: [{ firstHour: 100, lastHour: 99 }] }));
});

test('adapter fills nullable start, country uncertainty, arbitrary loss values and three measures', () => {
  const data = reply(); data.rows[0].runs[0].row.sides[0].countriesUnknown = 'Countries not recorded';
  data.rows[0].runs[0].row.sides[0].losses.push({ label: 'Mod loss type', value: null, unknown: 'Not in this recording' });
  validateNavalBattlesReply(data); const [combat, result] = rows(data, 105);
  assert(rows(data, 105).every(isBattleRow));
  assert.deepEqual(combat.start, { hour: 100, bound: 'by' }); assert.equal(combat.end, null);
  assert.equal(combat.observedHour, 105); assert.equal(combat.windowIndex, 0);
  assert.deepEqual(combat.sides[0].countries, [{ tag: 'opaque mod country', hour: 105 }]);
  assert.equal(combat.sides[0].countriesUnknown, 'Countries not recorded');
  assert.equal(combat.sides[0].losses.at(-1)?.value, null);
  assert.deepEqual(combat.measures.map(m => m.label), ['Ships sunk', 'Peak ships', 'Length']);
  assert.deepEqual(navalBattleSize(combat, 'Length'), { label: 'Length', value: 5, atLeast: true });
  assert.equal(result.start, null); assert.deepEqual(result.end, { hour: 99, bound: 'exact' });
  for (const measure of ['Peak ships', 'Length'] as const) assert.deepEqual(navalBattleSize(result, measure), { label: measure, value: null, unknown: 'Not recorded' });
});

test('only complete current combat reads show; retired results require their loaded history window', () => {
  const data = reply(); assert.equal(rows(data, 109).length, 2);
  assert.deepEqual(rows(data, 110).map(r => r.id), ['result:800']);
  const later = { ...reply([], 110, 119), window: 1 };
  assert.deepEqual(navalBattleRows(new Map([[1, later]]), 115, () => true, true), [], 'later replies carry no retired inventory');
  assert.deepEqual(navalBattleRows(new Map([[1, later], [0, data]]), 115, () => true, true).map(r => r.id), ['result:800']);
});

test('result merges and prefetched windows do not backfill future countries, winner or sunk figures', () => {
  const old = reply([history('result:800', true)]); const changed = run(110, 119, true);
  changed.row.end = { hour: 110, bound: 'exact' }; changed.row.place = { name: null, hour: 110 };
  changed.row.winner = { side: 0 }; changed.row.sides[0].countries = [{ tag: 'new country', hour: 119 }];
  changed.measures[0] = { label: 'Ships sunk', value: 9 };
  const next = { ...reply([{ ...old.rows[0], runs: [changed] }], 110, 119), window: 1 };
  validateNavalBattlesReply(next); const windows = new Map([[1, next], [0, old]]);
  assert.deepEqual(navalBattleRows(windows, 108, () => true, true)[0].winner, { side: 1 });
  const shown = navalBattleRows(windows, 112, () => true, true)[0];
  assert.deepEqual(shown.winner, { side: 0 }); assert.equal(navalBattleSize(shown, 'Ships sunk').value, 9);
  assert.deepEqual(shown.sides[0].countries, [{ tag: 'new country', hour: 112 }]);
});

test('server ranking preserves label-keyed measures and order; unknown is never recomputed as zero', () => {
  assert.equal(NAVAL_BATTLE_DEFAULT_MEASURE, 'Ships sunk');
  const data: NavalBattlesRankingReply = { rows: [history('result:800', true), history()], censusHour: 109,
    effectiveMeasure: 'Peak ships', warnings: [], contentWarnings: [] };
  assert.equal(validateNavalBattlesRanking(data), data);
  assert.deepEqual(rankedNavalBattleRows(data, 109).map(r => r.id), ['result:800', 'combat:900:100']);
  data.rows[0].runs[0].measures.reverse();
  assert.equal(navalBattleSize(rankedNavalBattleRows(data, 109)[0], 'Ships sunk').value, 1);
  assert.equal(navalBattleSize(rankedNavalBattleRows(data, 109)[0], 'Peak ships').value, null);
  assert.deepEqual(rankedNavalBattleRows(undefined, 109), []);
  assert.throws(() => validateNavalBattlesRanking({ ...data, effectiveMeasure: 'missing' }));
});

test('recorded ended measures have as-recorded wording; unproved links stay Not recorded', () => {
  const result = rows(reply())[1];
  assert.equal(navalBattleMeasureText(navalBattleSize(result, 'Peak ships'), result), 'Not recorded');
  assert.equal(navalBattleMeasureText({ label: 'Peak ships', value: 9 }, result), '9');
  assert.equal(navalBattleMeasureText({ label: 'Length', value: 5 }, result), '5 h');
});

test('linked result length and peak stay at the observed reads rather than extending to later result reads', () => {
  const data = reply([history('result:800', true)], 107, 109);
  const result = data.rows[0].runs[0];
  result.firstHour = 107;
  result.row.start = { hour: 100, bound: 'by' };
  result.row.place = { hour: 100, name: null };
  result.row.end = { hour: 107, bound: 'exact' };
  result.peakHour = 104;
  result.measures = [{ label: 'Ships sunk', value: 1 }, { label: 'Peak ships', value: 8, atLeast: true }, { label: 'Length', value: 4, atLeast: true }];
  validateNavalBattlesReply(data);
  for (const at of [107, 109, 120]) {
    const shown = rows(data, at)[0];
    assert.equal(navalBattleSize(shown, 'Length').value, 4);
    assert.equal(navalBattleSize(shown, 'Peak ships').value, 8);
    assert.equal(shown.peakHour, 104);
    assert.equal(navalBattleMeasureText(navalBattleSize(shown, 'Length'), shown), '≥ 4 h');
  }
});

test('country filtering uses supplying hours and serves 5,000 opaque tags without a cap', () => {
  const data = reply([history('result:800', true)]); const tags = Array.from({ length: 5000 }, (_, i) => `mod country ${i}`);
  data.rows[0].runs[0].row.sides[0].countries = tags.map(tag => ({ tag, hour: 109 })); validateNavalBattlesReply(data);
  const windows = new Map([[0, data]]);
  const shown = navalBattleRows(windows, 120, (tag, hour) => tag === tags[4999] && hour === 109, false);
  assert.equal(shown.length, 1); assert.equal(shown[0].sides[0].countries.length, 5000);
  assert.equal(navalBattleRows(windows, 120, (_, hour) => hour === 120, false).length, 0);
  data.rows[0].runs[0].row.sides[1].countriesUnknown = 'Countries not recorded';
  assert.equal(navalBattleRows(windows, 120, () => false, false).length, 1);
});

test('generic place helpers name retained results as of a later recorded read, stably across live ticks', () => {
  const shown = rows(reply(), 105);
  assert.deepEqual(battlePlaceRequests(shown), [{ provinceId: 45, hour: 100 }]);
  assert.deepEqual(battlePlaceRequests(shown), battlePlaceRequests(rows(reply(), 109)));
  const places = validatePlaceNamesReply({ places: [{ provinceId: 45, hour: 100, name: 'Renamed mod sea zone', missing: null }], states: [], warnings: [] });
  const named = namedBattles(shown, places);
  assert(named.every(isBattleRow)); assert.equal(named[1].place.name, 'Renamed mod sea zone');
  assert.equal(named[1].place.hour, 99); assert.equal(named[1].place.namedAt, 100);
  assert.equal(named[1].sides[0].countries[0].hour, 105);
});

test('warnings and census dates are scoped to the requested recorded window', () => {
  const data = reply(); data.warnings = [{ hour: 101, reason: 'Failed members', count: 3 }, { hour: 108, reason: 'Failed links', count: 1 }];
  const windows = new Map([[0, data]]);
  assert.deepEqual(navalBattleWarnings(windows, 102, 109), ['Hour 108: Failed links (1)']);
  assert.equal(shownNavalCensusHour(windows, 105), 105); assert.equal(shownNavalCensusHour(windows, 120), 109);
});

test('all three replies preserve content diagnostics and refuse malformed diagnostics', () => {
  const contentWarnings = ['Mod content unavailable'];
  const data = { ...reply(), contentWarnings };
  const manifest: NavalBattlesManifestReply = { windows: [], available: false, unavailable: 'Not in this recording', contentWarnings };
  const ranking: NavalBattlesRankingReply = { rows: data.rows, censusHour: 109, effectiveMeasure: 'Ships sunk', warnings: [], contentWarnings };
  for (const [validate, value] of [[validateNavalBattlesReply, data], [validateNavalBattlesManifest, manifest],
    [validateNavalBattlesRanking, ranking]] as const) {
    assert.equal(validate(value).contentWarnings, contentWarnings);
    for (const malformed of [undefined, null, 'warning', [1]]) {
      assert.throws(() => validate({ ...value, contentWarnings: malformed }), /malformed/);
    }
  }
  const later = { ...reply([], 110, 119), window: 1, contentWarnings };
  assert.deepEqual(navalBattleWarnings(new Map([[1, later], [0, data]]), 110, 119), contentWarnings);
});

test('the log renders at window starts, reads and each battle run edge, never hourly', () => {
  const run = (firstHour: number, lastHour: number) => ({ firstHour, lastHour }) as NavalBattleRun;
  const window = { window: 0, interval: { firstHour: 100, lastHour: 819 }, observed: [[100, 400], [420, 819]] as [number, number][],
    rows: [{ id: 'combat:1:110', provinceId: 5, runs: [run(110, 150), run(151, 300)] }] as NavalBattleHistory[],
    warnings: [], contentWarnings: [] } satisfies NavalBattlesReply;
  const changes = navalBattleChanges(new Map([[0, window]]));
  assert.deepEqual(battleChangeHours(changes), [100, 110, 151, 301, 420]);
  assert.deepEqual([...changes.whole].sort((a, b) => a - b), [100, 420], 'every battle where a window begins or is read');
  assert.deepEqual([...changes.battles.keys()].sort((a, b) => a - b), [110, 151, 301], 'the battle alone at its runs\' edges');
  assert.throws(() => validateNavalBattlesManifest({ windows: [], available: false, unavailable: null, contentWarnings: [] }),
    'an unavailable manifest names why');
});

test('per-run province ids stay unrecorded on early runs, name later provinces at their observed hour, and update in follow playback', () => {
  const early = run(100, 104, false, null);
  const late = run(105, 109, false, 45);
  late.row.place.namedAt = 105;
  const data = reply([{ id: 'combat:900:100', provinceId: 45, runs: [early, late] }]);
  validateNavalBattlesReply(data);
  const windows = new Map([[0, data]]);
  assert.deepEqual(navalWindowPlaces(windows), [{ provinceId: 45, hour: 105 }]);
  const before = navalBattleRows(windows, 102, () => true, true)[0];
  const after = navalBattleRows(windows, 107, () => true, true)[0];
  assert.equal(before.provinceId, null);
  assert.equal(after.provinceId, 45);
  assert.equal(after.place.hour, 100);
  assert.equal(after.place.namedAt, 105);
  assert.equal(battleJumpHour(after), 100);
  const labels = { place: 'Battle', date: 'Date', start: 'Started', end: 'Ended', winner: 'Winner',
    ongoing: 'Ongoing', unknownPlace: 'Unnamed province', unrecordedPlace: 'Province not recorded', unknownStart: 'Start not recorded' };
  assert.equal(battlePlaceText(before, labels, true), 'Province not recorded');
  assert.equal(battleTooltip(before, labels, tag => ({ name: tag })).title, 'Province not recorded');
  assert.equal(battlePlaceText(after, labels, false), 'Unnamed province');

  const known = run(100, 104, false, 500);
  const dropped = run(105, 109, false, null);
  const dropWindows = new Map([[0, reply([{ id: 'combat:900:100', provinceId: 500, runs: [known, dropped] }])]]);
  assert.equal(navalBattleRows(dropWindows, 102, () => true, true)[0].provinceId, 500);
  assert.equal(navalBattleRows(dropWindows, 107, () => true, true)[0].provinceId, null);
  const states = battleStateRows([100, 105], at => navalBattleRows(dropWindows, at, () => true, true), true, navalBattleState);
  assert.equal(states.length, 2);
  assert.equal(states[0].item.provinceId, 500);
  assert.equal(states[1].revealedHour, 105);
  assert.equal(states[1].item.provinceId, null);
});

test('naval side summaries show convoys on convoy raids and escorted convoys without calling them 0 ships', () => {
  const side = (counts: NavalBattleRun['row']['sides'][number]['counts']) => ({
    label: 'Defenders', countries: [], counts, losses: [],
  });
  assert.equal(navalSideText(side([{ label: 'Ships', value: 6 }, { label: 'Convoys', value: 0 }]), false), '6 ships');
  assert.equal(navalSideText(side([{ label: 'Ships', value: 6 }, { label: 'Convoys', value: 0 }]), true), '6');
  assert.equal(navalSideText(side([{ label: 'Ships', value: 0 }, { label: 'Convoys', value: 7 }]), false), '7 convoys');
  assert.equal(navalSideText(side([{ label: 'Ships', value: 0 }, { label: 'Convoys', value: 7 }]), true), '7c');
  assert.equal(navalSideText(side([{ label: 'Ships', value: 0 }, { label: 'Convoys', value: 1 }]), false), '1 convoy');
  assert.equal(navalSideText(side([{ label: 'Ships', value: 13 }, { label: 'Convoys', value: 1 }]), false), '13 ships, 1 convoy');
  assert.equal(navalSideText(side([{ label: 'Ships', value: 13 }, { label: 'Convoys', value: 1 }]), true), '13s+1c');
  assert.equal(navalSideText(side([{ label: 'Ships', value: null, unknown: 'Not recorded' }, { label: 'Convoys', value: 2 }]), false), 'Not recorded ships, 2 convoys');
});
