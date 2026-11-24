import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseAnswer, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { Spec } from '../../dashboard/follow/messages.fixture.ts';
import { createBoundedReadView, emptyReadView } from '../../families/boundedView.ts';
import { deriveTableTooltip, resolveCellAtHour, resolveTableRows } from '../../families/table/model.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import type { ApmReply } from '../../generated/dashboard';
import { apmHeadRows } from './apmHead.ts';
import { apmTableColumns, apmTableProjection } from './apmProjection.ts';

const bits = (value: number) => { const view = new DataView(new ArrayBuffer(8)); view.setFloat64(0, value, true); return view.getBigInt64(0, true); };
const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

function headAnswer(rows: readonly {
  readonly key: bigint;
  readonly id: string;
  readonly values: readonly (bigint | string | null)[];
}[]) {
  const cols = rows[0]?.values.length ?? 0;
  const texts: string[] = [];
  const held = new Map<string, number>();
  const intern = (s: string) => {
    let at = held.get(s);
    if (at === undefined) held.set(s, at = texts.push(s) - 1);
    return at;
  };
  const names: number[] = [];
  const rawVals: bigint[] = [];
  const states: number[] = [];
  for (const row of rows) {
    names.push(intern(row.id));
    for (const v of row.values) {
      if (v === null) { rawVals.push(0n); states.push(CELL.missing); }
      else if (typeof v === 'string') { rawVals.push(BigInt(intern(v))); states.push(CELL.value); }
      else { rawVals.push(v); states.push(CELL.value); }
    }
  }
  const bytes: number[] = [];
  const u8 = (v: number) => bytes.push(v & 0xff);
  const u32 = (v: number) => { for (let i = 0; i < 4; i++) u8(Number((BigInt(v) >> BigInt(8 * i)) & 0xffn)); };
  const u64 = (v: bigint) => { for (let i = 0; i < 8; i++) u8(Number((BigInt.asUintN(64, v) >> BigInt(8 * i)) & 0xffn)); };
  const pad = () => { while (bytes.length % 8) u8(0); };
  u8(1); u8(1); u8(0); u8(0); u32(rows.length); u32(cols); u32(texts.length);
  rows.forEach(r => u64(r.key));
  rows.forEach(r => u64(r.key >> 64n));
  rawVals.forEach(v => u64(v));
  states.forEach(s => u8(s));
  pad();
  for (let i = 0; i < rows.length; i++) u32(0xffff_ffff);
  pad();
  names.forEach(n => u32(n));
  pad();
  const enc = texts.map(t => new TextEncoder().encode(t));
  let end = 0;
  u32(0);
  for (const b of enc) { end += b.length; u32(end); }
  pad();
  for (const b of enc) b.forEach(u8);
  pad();
  return parseAnswer(new Uint8Array(bytes));
}

test('a followed eAPM reads back as the request path\'s series: rates, counts, arbitrary UTF-8 tags, dated names, and missing values with reasons', () => {
  const followed = headAnswer([
    { key: 0n, id: 'GER', values: [bits(2.35), bits(2.35), 47n, 3n, bits(20), 1n, 0n, 'GER', 160n, 'GER', null] },
    { key: 1n, id: 'user:ab', values: [null, bits(0.5), null, null, bits(0), 1n, 1n, 'CUSTOM_MOD_TAG_12345', 150n, 'DatedPlayer', null] },
    { key: 2n, id: 'user:cd', values: [bits(1.2), bits(1.2), 12n, 0n, bits(10), 1n, 0n, 'FRA', 150n, null, 'Player name is not valid UTF-8.'] },
  ]);
  const rows = apmHeadRows(followed);
  assert.deepEqual(rows[0], {
    id: 'GER', name: 'GER', nameMissing: null, tag: 'GER', hour: 160, points: [],
    summary: { apm: 2.35, confirmedApm: 2.35, orders: '47', pausedOrders: '3', unpausedMinutes: 20, complete: true, gaps: false },
  });
  assert.deepEqual(rows[1]!.summary, { apm: null, confirmedApm: 0.5, orders: null, pausedOrders: null, unpausedMinutes: 0, complete: true, gaps: true });
  assert.equal(rows[1]!.tag, 'CUSTOM_MOD_TAG_12345', 'tags longer than 8 bytes survive round-trip via text table');
  assert.equal(rows[1]!.name, 'DatedPlayer', 'dated player name comes from the head window');
  assert.equal(rows[2]!.name, null);
  assert.equal(rows[2]!.nameMissing, 'Player name is not valid UTF-8.');
  const refused = answer({ notRecorded: 'Player orders were not recorded in this campaign.' });
  assert.deepEqual(apmHeadRows(refused), []);
});

test('apm-table resolves country at row observation hour including hour 0, gives each field its own missing/partial meaning, checks membership gaps over the active window only, and keeps missing player names notRecorded without internal ids', () => {
  const reply: ApmReply = {
    available: true,
    warnings: [],
    players: [{ id: 'p1', name: 'RenamedLater', nameMissing: null, tags: ['GER'], runs: [{ firstHour: 0, lastHour: 200, tag: 'GER', name: 'RenamedLater', nameMissing: null }] }],
    series: [
      {
        id: 'p1',
        name: 'Alice',
        nameMissing: null,
        tag: 'GER',
        hour: 0,
        summary: { apm: null, confirmedApm: null, orders: '12', pausedOrders: null, unpausedMinutes: 0, complete: true, gaps: true },
        points: [],
      },
      {
        id: 'user:deadbeef',
        name: null,
        nameMissing: 'Player name is not valid UTF-8.',
        tag: 'CUSTOM_MOD_TAG_12345',
        hour: 0,
        summary: { apm: 4.5, confirmedApm: 4.5, orders: '45', pausedOrders: '0', unpausedMinutes: 10, complete: true, gaps: false },
        points: [],
      },
    ],
  };
  const labels = {
    GER: [{ from: 0, name: 'Weimar', flag: 'old.png' }, { from: 50, name: 'Reich', flag: 'new.png' }],
    CUSTOM_MOD_TAG_12345: [{ from: 0, name: 'Custom Realm', flag: 'mod.png' }],
  };
  const opts = {
    players: true,
    showPaused: true,
   
    rankMarkers: false,
    labels,
    unobserved: [[10, 20]] as readonly (readonly [number, number])[],
    branchFirst: 0,
    windowHours: 168,
    staticSpan: [300, 400] as readonly [number, number],
  };
  const model = apmTableProjection.project(createBoundedReadView({ hour: 400, panels: new Map([['main', reply]]) }), opts);
  assert.equal(model.empty, undefined, 'early unobserved gap [10,20] does not suppress range [300,400]');
  const row = model.rows.find(r => r.key === 'p1')!;
  assert.deepEqual(row.cells.name!.value, { kind: 'known', value: 'Alice' }, 'M2: uses window dated name Alice, never branch-end RenamedLater');
  const resolvedCountry = resolveCellAtHour(row.cells.country!, 400);
  assert.deepEqual(resolvedCountry.value, { kind: 'known', value: 'Weimar' }, 'country cell resolves at observation hour 0, not window end 400');
  assert.equal(resolvedCountry.icon, 'old.png', 'flag resolves at observation hour 0');

  assert.equal(row.cells.apm!.value.kind, 'notApplicable', 'zero unpaused minutes makes rate notApplicable');
  assert.equal(row.cells.paused!.value.kind, 'notRecorded', 'unknown paused orders never inherits rate notApplicable');
  assert.equal(row.cells.orders!.value.kind, 'partial', 'orders carry partial state when summary.gaps is true');
  assert.equal(row.cells.minutes!.value.kind, 'partial', 'unpausedMinutes carries partial state when summary.gaps is true');

  const missingRow = model.rows.find(r => r.key === 'user:deadbeef')!;
  assert.deepEqual(missingRow.cells.name!.value, { kind: 'notRecorded', reason: 'Player name is not valid UTF-8.' }, 'S12: missing player name stays notRecorded with reason, never internal id');
  assert.equal(missingRow.name, 'Name not recorded');
  assert.equal(missingRow.tooltipTitle, 'Name not recorded');
  assert.deepEqual(resolveCellAtHour(missingRow.cells.country!, 400).value, { kind: 'known', value: 'Custom Realm' }, 'M3: long UTF-8 tag resolves its country look');

  const followed = headAnswer([
    { key: 0n, id: 'p1', values: [null, null, 12n, null, bits(0), 1n, 1n, 'GER', 0n, 'Alice', null] },
    { key: 1n, id: 'user:deadbeef', values: [bits(4.5), bits(4.5), 45n, 0n, bits(10), 1n, 0n, 'CUSTOM_MOD_TAG_12345', 0n, null, 'Player name is not valid UTF-8.'] },
  ]);
  const followedModel = apmTableProjection.project(createBoundedReadView({ hour: 400, answers: new Map([['main', followed]]) }), opts);
  assert.deepEqual(followedModel.rows.find(r => r.key === 'p1')!.cells.name!.value, { kind: 'known', value: 'Alice' });
  assert.deepEqual(followedModel.rows.find(r => r.key === 'user:deadbeef')!.cells.name!.value, { kind: 'notRecorded', reason: 'Player name is not valid UTF-8.' });

  const cols = apmTableColumns(true, true);
  const tip = deriveTableTooltip(resolveTableRows(model, tableSettingsOf(undefined, cols, { density: 'comfortable', sort: { column: 'apm', direction: 'descending' } })), 'p1', cols, 400)!;
  assert.equal(tip.rows.find(r => r.label === 'Country')?.valueState?.text, 'Weimar');

  const followOpts = { ...opts, staticSpan: null };
  assert.equal(apmTableProjection.nextChange?.(emptyReadView(5), followOpts), 10, 'trailing window enters gap at 10');
  assert.equal(apmTableProjection.nextChange?.(emptyReadView(15), followOpts), 50, 'a country name that changes at 50 bounds the model first');
  assert.equal(apmTableProjection.nextChange?.(emptyReadView(55), followOpts), 188, 'trailing window leaves gap [10,20] at 20 + 168 = 188');
  assert.equal(apmTableProjection.project(createBoundedReadView({ hour: 180, panels: new Map([['main', reply]]) }), followOpts).emptyKind, 'unavailable', 'trailing [13,180] overlaps [10,20]');
  assert.equal(apmTableProjection.project(createBoundedReadView({ hour: 188, panels: new Map([['main', reply]]) }), followOpts).empty, undefined, 'trailing [21,188] is past [10,20]');
});
