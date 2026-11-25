import assert from 'node:assert/strict';
import test from 'node:test';
import { battleChangeHours, battleChanges, battleJumpHour, battleCountryRequests, battleDate, battleDateMoment, battleFlagCount, battleShortDate, battleLogRows, battleState, battleStateRows, heldWindowsEnd, battleTooltip, battleValueText, battleWinner, isBattleRow, BATTLE_PARTIAL_REASON, battlePartial, battleValueRow, battleValueTitle, markPartial,
  battlePlaceRequests, heldAtHour, namedBattles, rowsById } from './battleRows.ts';
import { validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import type { BattleValue } from '../../generated/dashboard';
import type { BattleLabels, BattleRow } from './battleRows';

const labels: BattleLabels = { place: 'Location', date: 'Date', start: 'Start', end: 'End', winner: 'Winner',
  ongoing: 'Ongoing', unknownPlace: 'Location unknown', unknownStart: 'Start not recorded' };

const row = (id = 'episode', end: BattleRow['end'] = { hour: 30, bound: 'by' }): BattleRow => ({ id,
  place: { name: 'Modded location', hour: 29 }, start: { hour: 10, bound: 'by' }, end,
  sides: [
    { label: 'First side', countries: [{ tag: 'opaque tag/a', hour: 29 }], counts: [{ label: 'Ships', value: 3 }], losses: [{ label: 'Sunk', value: null, unknown: 'Losses not read' }] },
    { label: 'Other side', countries: [{ tag: 'opaque tag/a', hour: 10 }, { tag: 'opaque tag/b', hour: 29 }], counts: [{ label: 'Ships', value: 2 }], losses: [] },
  ], winner: { unknown: 'Result not recorded' } });

test('later naming reads preserve battle dates, request stable hours and date the tooltip', () => {
  const battle = { ...row(), provinceId: 17, place: { name: null, hour: 29, namedAt: 40 } };
  assert.equal(isBattleRow(battle), true, 'end may precede a later naming read');
  assert.equal(isBattleRow({ ...battle, place: { ...battle.place, namedAt: 28 } }), false);
  assert.equal(isBattleRow({ ...battle, place: { ...battle.place, namedAt: 29 } }), false);
  const ordinary = { ...battle, place: { name: null, hour: 29 } };
  assert.deepEqual(battlePlaceRequests([battle, ordinary, battle]), [{ provinceId: 17, hour: 29 }, { provinceId: 17, hour: 40 }]);
  const places = validatePlaceNamesReply({ places: [{ provinceId: 17, hour: 40, name: 'Mod sea zone', missing: null }], states: [], warnings: [] });
  const named = namedBattles([battle, ordinary], places);
  assert.equal(named[0].place.name, 'Mod sea zone'); assert.equal(named[0].place.hour, 29);
  assert.equal(named[1], ordinary);
  assert.equal(battleTooltip(named[0], labels, tag => ({ name: tag })).rows.some(row => row.label.startsWith('named as of ')), false);
  assert.throws(() => validatePlaceNamesReply({ ...places, places: [places.places[0], places.places[0]] }));
});


test('short table dates use the start bound; unrecorded starts use a known end', () => {
  const battle = row();
  assert.equal(battleDateMoment(battle), battle.start);
  assert.equal(battleShortDate(battle, labels.unknownStart), 'by Hour 10');
  assert.equal(battleShortDate({ ...battle, start: { hour: 10, bound: 'exact' } }, labels.unknownStart), 'Hour 10');
  assert.equal(battleShortDate({ ...battle, start: null }, labels.unknownStart), 'by Hour 30');
  assert.equal(battleShortDate({ ...battle, start: null, end: null }, labels.unknownStart), labels.unknownStart);
});

test('flag display capacity fits measured space and reserves a counted overflow marker', () => {
  assert.equal(battleFlagCount(0, 100, 18, 2, 20), 0);
  assert.equal(battleFlagCount(2, 38, 18, 2, 20), 2);
  assert.equal(battleFlagCount(3, 38, 18, 2, 20), 0);
  assert.equal(battleFlagCount(3, 40, 18, 2, 20), 1);
  assert.equal(battleFlagCount(5000, 100, 18, 2, 35), 3);
  assert.equal(battleFlagCount(5000, 0, 18, 2, 35), 0);
  assert.equal(battleFlagCount(5000, 100000, 18, 2, 35), 5000);
});

test('battle rows retain bounds, missing starts, unknown results and explicit zero counts', () => {
  const battle = row();
  assert.equal(isBattleRow(battle), true);
  assert.match(battleDate(battle.start, labels.unknownStart), /^by /);
  assert.equal(battleWinner(battle), 'Result not recorded');
  assert.equal(battleValueText({ label: 'Ships', value: 0 }), '0');
  const sunk = { label: 'Sunk', value: 3, atLeast: true };
  assert.deepEqual(battleValueRow('Sunk', sunk, '3 so far', { indent: true }),
    { indent: true, label: 'Sunk', valueState: { kind: 'partial', text: '3 so far', reason: BATTLE_PARTIAL_REASON } }, 'R2-K7: a partial battle figure keeps its mark and reason');
  assert.deepEqual(battleValueRow('Ships', { label: 'Ships', value: 5 }, '5'), { label: 'Ships', value: '5' });
  assert.equal(markPartial('4 h', battlePartial({ label: 'Length', value: 4, atLeast: true })), '4 h partial');
  assert.equal(battleValueTitle(sunk, '3'), `3, partial: ${BATTLE_PARTIAL_REASON}`);
  for (const value of [0.123456789, 1e-30]) {
    assert.equal(battleValueText({ label: 'Caller measure', value }), String(value));
  }
  const naval = { ...battle, start: null };
  assert.equal(isBattleRow(naval), true);
  const tooltip = battleTooltip(naval, labels, (tag, hour) => ({ name: `${tag} at ${hour}` }));
  assert.equal(tooltip.rows[0].value, 'Start not recorded');
  assert.match(tooltip.rows[1].value!, /^by /);
  assert.equal(tooltip.rows.at(-1)?.value, 'Result not recorded');
  assert.equal(isBattleRow({ ...battle, winner: { side: 8 } }), false);
  assert.equal(isBattleRow({ ...battle, winner: { side: 0, unknown: 'Guess' } }), false);
  const ambiguous = row();
  ambiguous.sides[0].losses = [{ label: 'Ships', value: 1 }];
  assert.equal(isBattleRow(ambiguous), false);
  assert.equal(battleWinner({ ...battle, winner: { side: 1 } }), 'Other side');
});

test('Live pins ongoing battles while stable IDs and by dates survive ordering changes', () => {
  const older = row('ongoing', null);
  const newer = row('ended', { hour: 40, bound: 'by' });
  assert.deepEqual(battleLogRows([newer, older], true).map(row => row.id), ['ongoing', 'ended']);
  assert.deepEqual(battleLogRows([older, newer], false).map(row => row.id), ['ended', 'ongoing']);
  assert.equal(battleLogRows([newer], true)[0].by, true);
  assert.equal(battleLogRows([{ ...older, place: { ...older.place, hour: 100 } }], true)[0].key, 'ongoing');
});

test('only supplied visible rows ask for looks, independently at each recorded country hour', () => {
  const visible = row();
  const unseen = row('unseen');
  unseen.sides[0].countries = [{ tag: 'not drawn', hour: 50 }];
  const wanted = battleCountryRequests([visible]);
  assert.deepEqual(wanted, [{ tag: 'opaque tag/a', hour: 29 }, { tag: 'opaque tag/b', hour: 29 }, { tag: 'opaque tag/a', hour: 10 }]);
  assert.ok(!wanted.some(country => country.tag === 'not drawn'));
});

test('unknown endings stay visible and Jump uses the start or supplying hour', () => {
  const battle = row('missing-end', { unknown: 'Ending not recorded' });
  assert.equal(isBattleRow(battle), true);
  assert.equal(battleDate(battle.end, labels.ongoing), 'Ending not recorded');
  assert.equal(battleLogRows([battle], true).length, 1);
  assert.equal(battleJumpHour(battle), 10);
  assert.equal(battleJumpHour(row()), 10);
  assert.equal(battleJumpHour({ ...battle, start: null }), 29);
  assert.equal(battleTooltip(battle, labels, tag => ({ name: tag })).rows[1].value, 'Ending not recorded');
});

test('5,000 countries in drawn rows keep independent fact requests', () => {
  const battle = row();
  battle.sides[0].countries = Array.from({ length: 5000 }, (_, id) => ({ tag: `synthetic/${id}`, hour: 29 }));
  battle.sides[1].countries = [];
  assert.equal(battleCountryRequests([battle]).length, 5000);
});

test('caller formatters receive the dated row for peak-hour text', () => {
  const battle = row();
  const tooltip = battleTooltip(battle, labels, tag => ({ name: tag }), [{ label: 'Ships', title: 'Peak size',
    value: row => row.sides[0]!.counts[0]!,
    format: (value: BattleValue, row: BattleRow) => `${battleValueText(value)} at ${row.place.hour}` }]);
  assert.equal(tooltip.rows[2].value, '3 at 29');
});

test('unknown side participants remain visibly distinct from no participants', () => {
  const battle = row();
  battle.sides[0].countriesUnknown = 'Countries not recorded';
  assert.equal(isBattleRow(battle), true);
  assert.equal(isBattleRow({ ...battle, sides: [{ ...battle.sides[0], units: [] }] }), false);
  assert.equal(isBattleRow({ ...battle, sides: [{ ...battle.sides[0], countriesUnknown: 1 }] }), false);
  const tooltip = battleTooltip(battle, labels, tag => ({ name: tag }));
  assert.ok(tooltip.rows.some(row => row.label === 'Countries unknown'));
});

test('caller loss and measure tooltip formatters preserve the shared row shape', () => {
  const battle = row('running', null);
  battle.sides[0].losses = [{ label: 'Caller loss', value: 1.25 }];
  const tip = battleTooltip(battle, labels, tag => ({ name: tag }), [{ label: 'Caller loss', title: 'Caller measure',
    value: () => battle.sides[0].losses[0], format: () => 'row text', tooltipFormat: () => 'dated measure' }], undefined,
  (value, row, side, tooltip) => `${battleValueText(value)} so far at ${row.place.hour}; side ${side}; tooltip ${tooltip}`);
  assert.equal(tip.rows[2].value, 'dated measure');
  assert.ok(tip.rows.some(part => part.label === 'Caller loss' && part.value === '1.25 so far at 29; side 0; tooltip true'));
  assert.ok(tip.rows.some(part => part.label === 'Ships' && part.value === '3'));
  assert.equal(isBattleRow(battle), true);
});

test('dated known-country flag failures stay distinct from pending flags and an unknown native participant', () => {
  const battle = row();
  battle.sides[0].countriesUnknown = 'Participant reference was not recorded';
  const reads: [string, number][] = [];
  const completed = { name: '', flagReason: 'Synthetic flag definition was refused', flagPending: false };
  const pending = { name: 'Pending country', flagPending: true };
  const tip = battleTooltip(battle, labels, (tag, hour) => {
    reads.push([tag, hour]);
    return hour === 10 ? pending : completed;
  });
  assert.deepEqual(reads, [['opaque tag/a', 29], ['opaque tag/a', 10], ['opaque tag/b', 29]]);
  assert.equal(tip.rows.find(part => part.countryFlag === completed)?.countryFlag, completed);
  assert.equal(tip.rows.find(part => part.label === pending.name)?.countryFlag, pending);
  assert.equal(tip.rows.find(part => part.label === 'Countries unknown')?.countryFlag, undefined);
  assert.equal(tip.rows.find(part => part.countryFlag === completed)?.label, '', 'an authoritative empty name stays empty');
});

test('a battle log over a block holds each battle\'s states, each shown from its change to the next, in every hour\'s order', () => {
  const sides = (sunk: number | null): BattleRow['sides'] => [
    { label: 'First side', countries: [{ tag: 'mod tag', hour: 90 }], counts: [{ label: 'Ships', value: 3 }], losses: [sunk === null ? { label: 'Sunk', value: null, unknown: 'Losses not read' } : { label: 'Sunk', value: sunk }] },
    { label: 'Other side', countries: [{ tag: 'other mod tag', hour: 90 }], counts: [{ label: 'Ships', value: 2 }], losses: [] }];
  const battle = (id: string, start: number, end: number | null, sunk: number | null): BattleRow => ({ id, place: { name: null, hour: start },
    start: { hour: start, bound: 'exact' }, end: end === null ? null : { hour: end, bound: 'exact' }, sides: sides(sunk),
    winner: end === null ? { unknown: 'Result not recorded' } : { side: 0 } });
  const rowsAt = (hour: number) => [hour < 112 ? battle('x', 90, null, 1) : battle('x', 90, 112, 1),
    ...hour >= 105 ? [battle('y', 105, null, hour >= 115 ? null : hour >= 108 ? 2 : 1)] : []];
  const hours = [100, 105, 108, 112, 115];
  const shownAt = (rows: ReturnType<typeof battleStateRows<BattleRow>>, hour: number) => rows.filter(item =>
    Math.max(item.hour, item.revealedHour ?? item.hour) <= hour && (item.leftHour ?? Infinity) > hour).map(item => [item.item.id, item.item.end === null]);
  const rendered = battleStateRows(hours, rowsAt, true);
  for (let hour = 100; hour <= 120; hour++) {
    assert.deepEqual(shownAt(rendered, hour), battleLogRows(rowsAt(hour), true).map(item => [item.item.id, item.item.end === null]), `at ${hour}`);
  }
  assert.equal(rendered.filter(item => item.item.id === 'y').length, 3, 'losses a row renders: a state per read');
  const painted = battleStateRows(hours, rowsAt, true, item => battleState(item, true));
  assert.equal(painted.filter(item => item.item.id === 'y').length, 2, 'losses painted between changes: a state only when they stop being known');
  assert.ok(painted.every(item => item.id === item.item.id), 'a battle\'s states share its id: the log draws them as one row');
  const reads: string[] = [];
  const own = new Map([[105, ['y']], [108, ['y']], [112, ['x']], [115, ['y']]]);
  const byBattle = battleStateRows(hours, at => { reads.push(`*@${at}`); return rowsAt(at); }, true, undefined, at => own.get(at) ?? [],
    (id, at) => { reads.push(`${id}@${at}`); return rowsAt(at).find(item => item.id === id) ?? null; });
  assert.deepEqual(byBattle.map(item => [item.key, item.revealedHour ?? null, item.leftHour ?? null]), rendered.map(item => [item.key, item.revealedHour ?? null, item.leftHour ?? null]));
  assert.deepEqual(reads, ['*@100', 'y@105', 'y@108', 'x@112', 'y@115']);
  const changes = battleChanges([100, 100], [['x', [90, 112]], ['y', [105, 108, 115]], ['x', [112]]]);
  assert.deepEqual(changes.whole, [100]);
  assert.deepEqual(changes.battles.get(112), ['x'], 'a battle held by two windows changes once at an hour');
  assert.deepEqual(battleChangeHours(changes), [90, 100, 105, 108, 112, 115]);
});

test('a battle list draws ahead only through the windows held without a gap from the hour\'s, through the next window at most', () => {
  const windows = [{ firstHour: 100, lastHour: 130 }, { firstHour: 131, lastHour: 160 }, { firstHour: 161, lastHour: 190 }];
  assert.equal(heldWindowsEnd(windows, new Set([0]), 120), 131);
  assert.equal(heldWindowsEnd(windows, new Set([0, 1, 2]), 120), 161, 'never past the window after the hour\'s');
  assert.equal(heldWindowsEnd(windows, new Set([0, 1, 2]), 140), 191);
  assert.equal(heldWindowsEnd(windows, new Set([0, 2]), 120), 131, 'a window not held ends what may be drawn');
  assert.equal(heldWindowsEnd(windows, new Set([1]), 120), 121, 'the hour\'s own window not held: nothing past the hour');
  assert.equal(heldWindowsEnd(windows, new Set([0]), 99), 100, 'before the first window');
});

test('the rows at the head are replayed once an hour however many rows ask, and found by battle without a scan', () => {
  const replayed: number[] = [];
  const rowsAt = heldAtHour((hour: number) => { replayed.push(hour); return [{ id: `a${hour}` }, { id: `b${hour}` }] as unknown as BattleRow[]; });
  const first = rowsAt(10);
  for (let row = 0; row < 50; row++) assert.equal(rowsAt(10), first);
  assert.deepEqual(replayed, [10], 'one replay for the hour');
  assert.notEqual(rowsAt(11), first);
  assert.deepEqual(replayed, [10, 11], 'the head moving replays again');
  const byId = rowsById(rowsAt(11));
  assert.equal(byId, rowsById(rowsAt(11)), 'indexed once per list of rows');
  assert.equal(byId.get('b11'), rowsAt(11)[1]);
  assert.equal(byId.get('b10'), undefined);
});
