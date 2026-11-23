import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { Spec } from '../../dashboard/follow/messages.fixture.ts';
import { apmFromSort, apmHeadRows, apmHeadStructure, apmHolders, apmLiveValues, apmPausedView, apmSortValue, clockHead } from './apmHead.ts';

const bits = (value: number) => { const view = new DataView(new ArrayBuffer(8)); view.setFloat64(0, value, true); return view.getBigInt64(0, true); };
const packed = (tag: string) => [...tag].reduce((out, char, at) => out | BigInt(char.charCodeAt(0)) << BigInt(8 * at), 0n);
const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

test('a followed eAPM reads back as the request path\'s series: rates exactly, counts, tags, a missing value as missing with its reason', () => {
  const V = CELL.value, M = CELL.missing;
  const followed = answer({ keys: [0n, 1n], columns: 9, names: ['GER', 'user:ab'],
    values: [bits(2.35), bits(2.35), 47n, 3n, bits(20), 1n, 0n, packed('GER'), 160n,
      0n, bits(0.5), 0n, 0n, bits(0), 1n, 1n, packed('ITA'), 150n],
    states: [V, V, V, V, V, V, V, V, V, M, V, M, M, V, V, V, V, V] });
  const rows = apmHeadRows(followed);
  assert.deepEqual(rows[0], { id: 'GER', name: 'GER', tag: 'GER', hour: 160, points: [],
    summary: { apm: 2.35, confirmedApm: 2.35, orders: '47', pausedOrders: '3', unpausedMinutes: 20, complete: true, gaps: false } });
  assert.deepEqual(rows[1]!.summary, { apm: null, confirmedApm: 0.5, orders: null, pausedOrders: null, unpausedMinutes: 0, complete: true, gaps: true });
  assert.equal(rows[1]!.tag, 'ITA');
  assert.equal(apmHeadStructure(followed), 'GER:GER\u0000user:ab:ITA\u0000');
  const live = apmLiveValues(followed);
  assert.deepEqual(apmFromSort(live.get(liveKey('GER', 'apm')) as bigint), { apm: 2.35, gaps: false });
  assert.equal(live.get(liveKey('GER', 'orders')), 47n);
  assert.equal(live.get(liveKey('user:ab', 'apm')), undefined, 'an unknown rate is missing, never zero');
  assert.equal(live.missing(liveKey('user:ab', 'apm')), 'No unpaused time was observed in this period.');
  assert.equal(live.missing(liveKey('SOV', 'apm')), 'Not in the period');
  assert.ok((apmSortValue({ ...rows[0]!.summary, gaps: true }) as bigint) > (apmSortValue(rows[0]!.summary) as bigint), 'gaps ride below the rate, which orders the sort');
  assert.deepEqual(apmFromSort(apmSortValue({ ...rows[0]!.summary, apm: 7.1, gaps: true })!), { apm: 7.1, gaps: true });
  const refused = answer({ notRecorded: 'Player orders were not recorded in this campaign.' });
  assert.deepEqual(apmHeadRows(refused), []);
  assert.equal(apmLiveValues(refused).missing(liveKey('GER', 'apm')), 'Player orders were not recorded in this campaign.');
});

test('the followed clock reads its seconds exactly, an unclassified playing time as missing, a refusal as its reason', () => {
  const V = CELL.value, M = CELL.missing;
  assert.deepEqual(clockHead(answer({ keys: [0n], columns: 4, names: ['clock'], values: [bits(3723.25), bits(3000.5), bits(722.75), 1n], states: [V, V, V, V] })),
    { recorded: 3723.25, playing: 3000.5, paused: 722.75, complete: true });
  assert.deepEqual(clockHead(answer({ keys: [0n], columns: 4, names: ['clock'], values: [bits(10), 0n, 0n, 0n], states: [V, M, M, V] })),
    { recorded: 10, playing: null, paused: null, complete: false });
  assert.deepEqual(clockHead(answer({ notRecorded: 'Playing and paused time were not recorded in this campaign.' })), { reason: 'Playing and paused time were not recorded in this campaign.' });
  assert.equal(clockHead(undefined), null);
});

test("the graph's legend names each holder as at the hour shown while it follows the head, never as at the campaign's end", () => {
  const rows = [{ id: 'GER@1', tag: 'GER', hour: 100_000 }, { id: 'ETH@2', tag: 'ETH', hour: 3_000 }, { id: 'user:ab', tag: null, hour: 100_000 }];
  assert.deepEqual(apmHolders(rows, null), [{ key: 'GER@1', tag: 'GER', hour: 100_000 }, { key: 'ETH@2', tag: 'ETH', hour: 3_000 }]);
  assert.deepEqual(apmHolders(rows, 2_000), [{ key: 'GER@1', tag: 'GER', hour: 2_000 }, { key: 'ETH@2', tag: 'ETH', hour: 2_000 }]);
  assert.deepEqual(apmHolders(rows, 50_000), [{ key: 'GER@1', tag: 'GER', hour: 50_000 }, { key: 'ETH@2', tag: 'ETH', hour: 3_000 }]);
});

test('paused or after a seek the graph view ends at the head, never past it; while playing the chart window follows', () => {
  const first = 1000, last = 9000, week = 168;
  assert.deepEqual(apmPausedView(first, first, last, week), [first, first + 1], 'at the first hour, its one hour: never two weeks ahead');
  assert.deepEqual(apmPausedView(first + 50.6, first, last, week), [first, first + 50]);
  assert.deepEqual(apmPausedView(first + 4000, first, last, week), [first + 4000 - week, first + 4000]);
  assert.deepEqual(apmPausedView(last + 30, first, last, week), [last - week, last], 'a head past the branch ends at its last hour');
  assert.equal(apmPausedView(null, first, last, week), null);
});

import { EMPTY_SOURCES } from '../../families/contract.ts';
import { deriveTableTooltip, resolveCellAtHour, resolveTableRows } from '../../families/table/model.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { apmTableColumns, apmTableProjection } from './apmProjection.ts';
import type { ApmReply } from '../../generated/dashboard';

test('apm-table resolves country at row observation hour including hour 0, gives each field its own missing/partial meaning, and checks membership gaps over the active window only', () => {
  const reply: ApmReply = {
    available: true,
    warnings: [],
    players: [{ id: 'p1', name: 'Alice', tags: ['GER'] }],
    series: [
      {
        id: 'p1',
        name: 'Alice',
        tag: 'GER',
        hour: 0,
        summary: { apm: null, confirmedApm: null, orders: '12', pausedOrders: null, unpausedMinutes: 0, complete: true, gaps: true },
        points: [],
      },
    ],
  };
  const labels = {
    GER: [{ from: 0, name: 'Weimar', flag: 'old.png' }, { from: 50, name: 'Reich', flag: 'new.png' }],
  };
  const opts = {
    players: true,
    showPaused: true,
    cellBars: false,
    rankMarkers: false,
    playerNames: { p1: 'Alice' },
    labels,
    unobserved: [[10, 20]] as readonly (readonly [number, number])[],
    branchFirst: 0,
    windowHours: 168,
    staticSpan: [300, 400] as readonly [number, number],
  };
  const model = apmTableProjection.project({ answers: new Map(), panels: new Map([['main', reply]]), live: new Map() }, 400, opts);
  assert.equal(model.empty, undefined, 'early unobserved gap [10,20] does not suppress range [300,400]');
  const row = model.rows[0]!;
  const resolvedCountry = resolveCellAtHour(row.cells.country!, 400);
  assert.deepEqual(resolvedCountry.value, { kind: 'known', value: 'Weimar' }, 'country cell resolves at observation hour 0, not window end 400');
  assert.equal(resolvedCountry.icon, 'old.png', 'flag resolves at observation hour 0');

  assert.equal(row.cells.apm!.value.kind, 'notApplicable', 'zero unpaused minutes makes rate notApplicable');
  assert.equal(row.cells.paused!.value.kind, 'notRecorded', 'unknown paused orders never inherits rate notApplicable');
  assert.equal(row.cells.orders!.value.kind, 'partial', 'orders carry partial state when summary.gaps is true');
  assert.equal(row.cells.minutes!.value.kind, 'partial', 'unpausedMinutes carries partial state when summary.gaps is true');

  const cols = apmTableColumns(true, true, false);
  const tip = deriveTableTooltip(resolveTableRows(model, tableSettingsOf(undefined, cols, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'apm', direction: 'descending' } })), 'p1', cols, new Set(), 400)!;
  assert.equal(tip.rows.find(r => r.label === 'Country')?.value, 'Weimar');

  const followOpts = { ...opts, staticSpan: null };
  assert.equal(apmTableProjection.nextChange?.(EMPTY_SOURCES, 5, followOpts), 10, 'trailing window enters gap at 10');
  assert.equal(apmTableProjection.nextChange?.(EMPTY_SOURCES, 15, followOpts), 188, 'trailing window leaves gap [10,20] at 20 + 168 = 188');
  assert.equal(apmTableProjection.project({ answers: new Map(), panels: new Map([['main', reply]]), live: new Map() }, 180, followOpts).emptyKind, 'unavailable', 'trailing [13,180] overlaps [10,20]');
  assert.equal(apmTableProjection.project({ answers: new Map(), panels: new Map([['main', reply]]), live: new Map() }, 188, followOpts).empty, undefined, 'trailing [21,188] is past [10,20]');
});
