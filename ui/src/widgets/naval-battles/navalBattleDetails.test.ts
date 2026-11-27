import test from 'node:test';
import assert from 'node:assert/strict';
import { navalDetailTooltip, validateNavalBattleDetails } from './navalBattleDetails.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import { ART_STATUS_ROW, deriveRowTooltip, deriveTableMirror } from '../../families/table/model.ts';
import { navalBattleClock, navalBattlesProjection, type NavalBattleSubject } from './navalBattlesProjection.ts';
import type { NavalBattleDetail, NavalBattleLogRow, NavalBattlesTailReply } from '../../generated/dashboard';

const detail: NavalBattleDetail = { id: 'result:15', hour: 43, rosterHour: 8,
  fleets: [{ names: ['Recorded fleet'], unknown: null }, { names: [], unknown: 'Fleet name not recorded' }],
  namedShips: [{ sinkHour: 9, knownAt: 19, name: 'Recorded ship', className: 'Mod ship class', sinkingCause: { kind: 'notRecorded', reason: 'sinking cause was not recorded' } }], shipRead: 43, unavailable: null };
test('fleet and sinking details preserve names and sinking dates without supplying dates', () => {
  assert.equal(validateNavalBattleDetails({ rows: [detail], warnings: [] }).rows[0], detail);
  const rows = navalDetailTooltip(detail);
  assert.ok(rows.some(row => row.label === 'Recorded ship' && row.value === 'Mod ship class'));
  assert.ok(!rows.some(row => row.label === 'Fleet roster read'));
  assert.ok(!rows.some(row => row.label === 'Recorded at'));
  assert.ok(rows.some(row => row.label === 'Sunk'));
});

test('detail failures and missing fleets hide backend reasons without claiming no losses', () => {
  assert.deepEqual(navalDetailTooltip(undefined, new Error('private backend error')), [
    { label: 'Couldn’t load battle details.', detail: 'private backend error', tone: 'muted', divider: true },
  ]);
  assert.deepEqual(navalDetailTooltip(undefined), [{ label: 'Loading battle details…', detail: undefined, tone: 'muted', divider: true }]);
  const rows = navalDetailTooltip({ ...detail, fleets: [{ names: [], unknown: 'native field refused' }, { names: [], unknown: null }],
    shipRead: null, namedShips: [], unavailable: 'private table unavailable' });
  assert.ok(rows.some(row => row.label === 'Fleet details unavailable.'));
  assert.ok(rows.some(row => row.label === 'No fleets'));
  assert.ok(rows.some(row => row.label === 'Named ship losses unavailable.'));
  assert.ok(!rows.some(row => row.label === 'No named ship losses recorded.'));
  assert.doesNotMatch(JSON.stringify(rows), /private|native field/);
});
test('future ship reads and mismatched repeated detail identities refuse', () => {
  assert.throws(() => validateNavalBattleDetails({ rows: [{ ...detail, namedShips: [{ ...detail.namedShips[0], knownAt: 44 }] }], warnings: [] }));
  assert.throws(() => validateNavalBattleDetails({ rows: [detail, detail], warnings: [] }));
  assert.throws(() => validateNavalBattleDetails({ rows: [{ ...detail, rosterHour: 44 }], warnings: [] }));
});
test('all recorded fleet and ship names remain in tooltip content without a chosen cap', () => {
  const many = { ...detail, fleets: [{ names: Array.from({ length: 5000 }, (_, i) => `Fleet ${i}`), unknown: null }, detail.fleets[1]],
    namedShips: Array.from({ length: 5000 }, (_, i) => ({ ...detail.namedShips[0], name: `Ship ${i}` })) };
  const valid = validateNavalBattleDetails({ rows: [many], warnings: [] });
  const rows = navalDetailTooltip(valid.rows[0]);
  assert.ok(rows.some(row => row.label === 'Fleet 4999'));
  assert.ok(rows.some(row => row.label === 'Ship 4999'));
});

test('the followed naval table shows the supplying read, partial sinkings and IC lost, unrecorded sides and an ended battle without its winner', () => {
  const live: NavalBattleLogRow = { key: 'combat:9:10', hour: 10, province: null, countries: [[], []], countriesHour: [10, 10],
    readHour: 15, detailId: 'combat:9:10', detailHour: null, resultHour: null, lastLiveHour: null, endSeen: false, shipsSunk: [2, null], convoysSunk: [0, 0], icLost: [1992.5, null], ships: [3, 0], convoys: [0, 0], result: 'ongoing' };
  const head: NavalBattlesTailReply = { branchId: '1', total: '3', offset: '0', missing: [], warnings: [], nextChange: null,
    rows: [live, { ...live, key: 'combat:8:9', hour: 9, detailId: 'combat:8:9', result: 'unknown' }, { ...live, key: 'result:4', hour: 8, detailId: 'result:4', detailHour: 8, resultHour: 8, result: 'attackerWon' }] };
  const source = buildRowSourceFromHead('branch', head, undefined, 18)!;
  const view = createBoundedReadView({ hour: 18, rows: new Map([['main', source]]) });
  const model = (following: boolean) => navalBattlesProjection.project(view, { scope: 'test', following, byDate: true });
  const project = (following: boolean) => model(following).rows[0]!;
  const row = project(true);
  assert.equal(row.cells.item?.value.kind, 'pending');
  assert.equal(row.cells.action?.actionLabel, 'Watch', 'a battle with live hours offers Watch');
  assert.equal((row.meta as NavalBattleSubject).row.key, 'combat:9:10');
  const named = buildRowSourceFromHead('branch', { ...head, rows: head.rows.map(each => ({ ...each, countries: [[1], [2]] as [number[], number[]] })) }, undefined, 18)!;
  const unreadFlags = createBoundedReadView({ hour: 18, rows: new Map([['main', named]]),
    identity: { resolve: (kind, key, at) => ({ kind, key, holder: { status: 'ready', value: key }, name: { status: 'ready', value: kind === 'battle' ? 'Second Battle of Shale Strait' : 'Recorded country' },
      flagChoice: { status: 'ready', value: 'recorded_flag' }, validInterval: [at, at] as const, nextChangeAt: Infinity }) },
    art: { resolve: () => ({ status: 'failed', reason: 'flag file unreadable', retryable: true, retry: () => undefined }) as never } });
  for (const each of navalBattlesProjection.project(unreadFlags, { scope: 'test', following: true, byDate: true }).rows) {
    assert.doesNotThrow(() => structuredClone(each.meta), 'a row’s subject crosses the lane worker boundary, so it carries no art callbacks');
    assert.deepEqual(each.cells.item?.value, { kind: 'known', value: 'Second Battle of Shale Strait' });
  }
  assert.equal(model(true).rows[2]!.cells.action?.actionLabel, 'Watch', 'a battle held only as a result offers Watch on its result window');
  assert.match(model(true).columns.find(column => column.key === 'shipsSunk')!.partialReason!, /lower bound/);
  assert.match(model(true).columns.find(column => column.key === 'icLost')!.partialReason!, /lower bound/);
  const ended = model(true).rows[1]!.cells.status!;
  assert.deepEqual([ended.value, ended.statusKind], [{ kind: 'known', value: 'Ended' }, 'finished']);
  assert.deepEqual(ended.tooltipRows!.map(line => [line.label, line.value.kind]), [['Result', 'known'], ['Winner', 'notRecorded']]);
  const endedRow = model(true).rows[1]!;
  const found = { kind: 'data', key: endedRow.key, name: endedRow.name, row: endedRow, rank: 0, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 } as const;
  const lines = deriveRowTooltip(found, model(true).columns).rows.filter(line => line.lookupKey !== ART_STATUS_ROW);
  assert.deepEqual(lines.map(line => [line.label, line.rawValue?.kind]), [['Date', 'known'], ['Battle', 'pending'], ['Result', 'known'], ['Winner', 'notRecorded'],
    ['Attackers', 'notRecorded'], ['Ships', 'known'], ['Ships sunk', 'partial'], ['IC lost', 'partial'],
    ['Defenders', 'notRecorded'], ['Ships', 'zero'], ['Ships sunk', 'notRecorded'], ['IC lost', 'notRecorded']],
    'the tooltip groups the figures under each side instead of repeating the pair columns');
  assert.equal(deriveRowTooltip(found, model(true).columns).note, 'The game keeps a winner only for battles a player takes part in');
  assert.equal(deriveRowTooltip({ ...found, row: model(true).rows[0]! }, model(true).columns).note, undefined);
  assert.deepEqual(row.cells.shipsSunk!.pair!.map(value => value.kind), ['partial', 'notRecorded']);
  assert.equal(row.cells.shipsSunk!.value.kind, 'notRecorded');
  assert.deepEqual(row.cells.icLost!.pair!.map(value => value.kind), ['partial', 'notRecorded']);
  assert.equal(row.cells.icLost!.value.kind, 'notRecorded');
  assert.equal(row.cells.ships!.value.kind, 'known');
  assert.equal(row.cells.ships!.pair![1].kind, 'zero');
  assert.deepEqual(row.cells.country!.flags!.map(flag => flag.tag), ['?', '?']);
  assert.equal(row.live, true);
  assert.deepEqual(JSON.parse(row.tooltipScrollId!), ['test', 'combat:9:10', 15]);
  assert.deepEqual(JSON.parse(project(false).tooltipScrollId!), ['test', 'combat:9:10', 18]);
});

test('a naval battle plays its live hours, then its result window from the hour the result was first read, or an end note when no result was recorded', () => {
  const row: NavalBattleLogRow = { key: 'combat:9:10', hour: 10, province: null, countries: [[], []], countriesHour: [10, 10],
    readHour: 10, detailId: 'result:5', detailHour: 10, resultHour: 10, lastLiveHour: 10, endSeen: false, shipsSunk: [1, 0], convoysSunk: [0, 0], icLost: [5, 0], ships: [3, 2], convoys: [0, 0], result: 'attackerWon' };
  assert.deepEqual(navalBattleClock(row, 10, 0, 10), { liveEnd: 10, live: [[10, 10]], result: { handle: '5', from: 10 }, span: { first: 10, last: 10 }, noteFrom: null });
  assert.deepEqual(navalBattleClock({ ...row, detailHour: 25, resultHour: 20, lastLiveHour: 19 }, 40, 0, 40),
    { liveEnd: 19, live: [[10, 19]], result: { handle: '5', from: 20 }, span: { first: 10, last: 20 }, noteFrom: null });
  assert.deepEqual(navalBattleClock({ ...row, key: 'result:5', hour: 8, detailHour: 15, resultHour: 12, lastLiveHour: null }, 40, 0, 40),
    { liveEnd: null, live: [], result: { handle: '5', from: 12 }, span: { first: 12, last: 12 }, noteFrom: null }, 'a battle held only as a result opens on its result');
  const unresolved = { ...row, detailId: 'combat:9:10', detailHour: 19, resultHour: null, lastLiveHour: 19, result: 'unknown' } as const;
  assert.deepEqual(navalBattleClock({ ...unresolved, endSeen: true }, 40, 0, 40), { liveEnd: 19, live: [[10, 19]], result: null, span: { first: 10, last: 20 }, noteFrom: 20 });
  assert.equal(navalBattleClock({ ...unresolved, lastLiveHour: 10 }, 10, 0, 10).noteFrom, null, 'an end after the branch is never reached');
  assert.deepEqual(navalBattleClock({ ...unresolved, result: 'ongoing', lastLiveHour: null }, 14, 0, 40), { liveEnd: 14, live: [[10, 14]], result: null, span: { first: 10, last: 14 }, noteFrom: null });
  assert.equal(navalBattleClock({ ...unresolved, lastLiveHour: null }, 40, 0, 40).span, null);
});

test('convoy-only sides show convoy counts and live zero sinkings stay qualified as partial lower bounds', () => {
  const raid: NavalBattleLogRow = {
    key: 'combat:12:20', hour: 20, province: null, countries: [[], []], countriesHour: [20, 20],
    readHour: 22, detailId: 'combat:12:20', detailHour: null, resultHour: null, lastLiveHour: null, endSeen: false,
    ships: [6, 0], convoys: [0, 4], shipsSunk: [0, 0], convoysSunk: [0, 2], icLost: [0, 0], result: 'ongoing',
  };
  const escorted: NavalBattleLogRow = {
    key: 'result:42', hour: 15, province: null, countries: [[], []], countriesHour: [15, 15],
    readHour: 18, detailId: 'result:42', detailHour: 18, resultHour: 18, lastLiveHour: null, endSeen: false,
    ships: [4, 2], convoys: [0, 5], shipsSunk: [0, 0], convoysSunk: [0, 0], icLost: [0, 0], result: 'attackerWon',
  };
  const depletedConvoys: NavalBattleLogRow = {
    key: 'combat:13:19', hour: 19, province: null, countries: [[], []], countriesHour: [19, 19],
    readHour: 22, detailId: 'combat:13:19', detailHour: null, resultHour: null, lastLiveHour: null, endSeen: false,
    ships: [4, 0], convoys: [0, 0], shipsSunk: [0, null], convoysSunk: [0, 2], icLost: [0, null], result: 'ongoing',
  };
  const depletedWithShipLoss: NavalBattleLogRow = {
    ...depletedConvoys, key: 'combat:14:18', hour: 18, detailId: 'combat:14:18', shipsSunk: [1, null],
  };
  const head: NavalBattlesTailReply = { branchId: '1', total: '4', offset: '0', missing: [], warnings: [], nextChange: null, rows: [raid, escorted, depletedConvoys, depletedWithShipLoss] };
  const source = buildRowSourceFromHead('branch', head, undefined, 22)!;
  const view = createBoundedReadView({ hour: 22, rows: new Map([['main', source]]) });
  const model = navalBattlesProjection.project(view, { scope: 'test', following: true, byDate: true });
  const [raidRow, escortedRow, depletedRow, depletedShipLossRow] = model.rows;
  assert.deepEqual(raidRow!.cells.ships!.pair, [{ kind: 'known', value: 6 }, { kind: 'known', value: '4c' }]);
  assert.deepEqual(raidRow!.cells.shipsSunk!.pair, [
    { kind: 'partial', value: 0, reason: 'Observed losses so far', atLeast: true },
    { kind: 'partial', value: '2c', reason: 'Observed losses so far', atLeast: true },
  ]);
  assert.deepEqual(raidRow!.cells.shipsSunk!.splitBar?.map(segment => segment.fraction), [0, 1]);
  assert.deepEqual(escortedRow!.cells.ships!.pair, [{ kind: 'known', value: 4 }, { kind: 'known', value: '2s+5c' }]);
  assert.deepEqual(escortedRow!.cells.shipsSunk!.pair, [{ kind: 'zero', value: 0 }, { kind: 'zero', value: 0 }]);
  assert.equal(depletedRow!.cells.shipsSunk!.value.kind, 'notRecorded');
  assert.equal(depletedShipLossRow!.cells.shipsSunk!.value.kind, 'notRecorded');

  const foundRaid = { kind: 'data', key: raidRow!.key, name: raidRow!.name, row: raidRow!, rank: 0, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 } as const;
  const raidLines = deriveRowTooltip(foundRaid, model.columns).rows.filter(line => line.lookupKey !== ART_STATUS_ROW);
  assert.deepEqual(
    raidLines.map(line => [line.label, line.rawValue?.kind, line.rawValue && 'value' in line.rawValue ? line.rawValue.value : null]),
    [
      ['Date', 'known', 'Hour 20'],
      ['Battle', 'pending', null],
      ['Result', 'known', 'Ongoing'],
      ['Attackers', 'notRecorded', null],
      ['Ships', 'known', 6],
      ['Ships sunk', 'partial', 0],
      ['IC lost', 'partial', 0],
      ['Defenders', 'notRecorded', null],
      ['Ships', 'zero', 0],
      ['Convoys', 'known', 4],
      ['Ships sunk', 'partial', 0],
      ['Convoys sunk', 'partial', 2],
      ['IC lost', 'partial', 0],
    ],
  );
  const foundDepleted = { kind: 'data', key: depletedRow!.key, name: depletedRow!.name, row: depletedRow!, rank: 2, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 } as const;
  const depletedLines = deriveRowTooltip(foundDepleted, model.columns).rows.filter(line => line.lookupKey !== ART_STATUS_ROW);
  assert.deepEqual(
    depletedLines.slice(-6).map(line => [line.label, line.rawValue?.kind, line.rawValue && 'value' in line.rawValue ? line.rawValue.value : null]),
    [
      ['Defenders', 'notRecorded', null],
      ['Ships', 'zero', 0],
      ['Convoys', 'zero', 0],
      ['Ships sunk', 'notRecorded', null],
      ['Convoys sunk', 'partial', 2],
      ['IC lost', 'notRecorded', null],
    ],
  );
  const mixedHead: NavalBattlesTailReply = {
    branchId: '1', total: '2', offset: '0', missing: [], warnings: [], nextChange: null,
    rows: [
      { ...raid, key: 'combat:15:20', countries: [[1, 2], [3]] },
      { ...raid, key: 'combat:16:20', countries: [[1], [2]] },
    ],
  };
  const mixedView = createBoundedReadView({
    hour: 22,
    rows: new Map([['main', buildRowSourceFromHead('branch', mixedHead, undefined, 22)!]]),
    identity: {
      resolve(kind, key) {
        const name = key === '\u00001'
          ? { status: 'pending' as const }
          : key === '\u00002'
            ? { status: 'absent' as const, reason: 'Country name unavailable' }
            : { status: 'ready' as const, value: 'Bravo' };
        return { kind, key, holder: { status: 'ready', value: key }, name, colour: { status: 'absent', reason: 'none' }, flagChoice: { status: 'absent', reason: 'none' }, validInterval: [0, Infinity] as const, nextChangeAt: Infinity };
      },
    },
  });
  const mixedModel = navalBattlesProjection.project(mixedView, { scope: 'test', following: true, byDate: true });
  const [mixedRow, pendingAbsentRow] = mixedModel.rows;
  const mixedFound = { kind: 'data', key: mixedRow!.key, name: mixedRow!.name, row: mixedRow!, rank: 0, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 } as const;
  const mixedTip = deriveRowTooltip(mixedFound, mixedModel.columns).rows;
  assert.deepEqual(mixedTip.find(line => line.label === 'Attackers')?.rawValue, { kind: 'pending', reason: 'Name is loading; Country name unavailable' });
  assert.equal(JSON.stringify(mixedTip).includes('\u0000'), false);
  assert.deepEqual(pendingAbsentRow!.cells.country!.value, { kind: 'pending', reason: 'Name is loading; Country name unavailable' });
  const pendingAbsentMirror = deriveTableMirror([{ kind: 'data', key: pendingAbsentRow!.key, name: pendingAbsentRow!.name, row: pendingAbsentRow!, rank: 1, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 }], mixedModel.columns, false);
  assert.equal(pendingAbsentMirror.rows[0]!.cells.find(cell => cell.column === 'country')?.spoken, 'Loading: Name is loading; Country name unavailable');
});
