import assert from 'node:assert/strict';
import test from 'node:test';
import { battleJumpHour, battleCountryRequests, battleDate, battleDateMoment, battleFlagCount, battleShortDate, battleLogRows, battleMeasure, battleTooltip, battleValueText, battleWinner, isBattleRow,
  battlePlaceRequests, namedBattles } from './battleRows.ts';
import { validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import type { BattleLabels, BattleRow, BattleValue } from './battleRows';
import { HOUR_EPOCH, gameDate, formatDate } from '../../dashboard/calendar.ts';

const labels: BattleLabels = { place: 'Location', date: 'Date', start: 'Start', end: 'End', winner: 'Winner',
  ongoing: 'Ongoing', unknownPlace: 'Location unknown', unknownStart: 'Start not recorded' };

test('battle dates show one day while retaining honest by bounds', () => {
  const hour = HOUR_EPOCH + 1936 * 365 * 24 + 8;
  const day = formatDate(gameDate(hour)!);
  assert.equal(battleDate({ hour, bound: 'exact' }, 'Unknown'), day);
  assert.equal(battleDate({ hour, bound: 'by' }, 'Unknown'), `by ${day}`);
});
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
  assert.deepEqual(battleMeasure(battle, 'Ships', 'Not read'), { label: 'Ships', value: 5, atLeast: false });
  assert.deepEqual(battleMeasure(battle, 'Sunk', 'Not read'), { label: 'Sunk', value: null, unknown: 'Losses not read; Not read' });
  assert.equal(battleValueText({ label: 'Ships', value: 0 }), '0');
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
    value: row => battleMeasure(row, 'Ships', 'Not read'),
    format: (value: BattleValue, row: BattleRow) => `${battleValueText(value)} at ${row.place.hour}` }]);
  assert.equal(tooltip.rows[2].value, '5 at 29');
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
