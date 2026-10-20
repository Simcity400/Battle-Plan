import assert from 'node:assert/strict';
import test from 'node:test';
import { sortRows } from '../../charts/table/logic.ts';
import { deployedAirByTag, EXILE_VALUES, exilesAt, exileTooltip, exileWarnings, legitimacyShare, legitimacyText, manpowerText, readNote, validateExileDetails, validateExilesReply } from './governmentsInExileData.ts';
import type { ExileRow } from './governmentsInExileData.ts';

type Run = { firstHour: number; lastHour: number; value: string };
const run = (firstHour: number, lastHour: number, value: string): Run => ({ firstHour, lastHour, value });
const exile = (tag: string, fields: Partial<Record<'host' | 'legitimacy' | 'manpower' | 'cap' | 'leaders' | 'read', Run[]>>, tags = [{ hour: 0, tag }]) =>
  ({ tags, host: fields.host ?? [], legitimacy: fields.legitimacy ?? [], manpower: fields.manpower ?? [], cap: fields.cap ?? [], leaders: fields.leaders ?? [], read: fields.read ?? [] });
const reply = (exileHistory: ReturnType<typeof exile>[], holders: Record<string, { hour: number; tag: string }[]>, lastHour: number | null = 10_000) => ({
  branchId: '1', mode: 'exiles', lastHour, tag: null, hour: null, balances: [], shown: [], elections: [], exiles: [], reached: [], names: {},
  exileHistory, holders, known: [{ firstHour: 0, lastHour: 10_000 }], warnings: [] as string[] });
const all = () => true;
const keyOf = (row: ExileRow) => row.key;
const label = (name: string) => ({ name });

test('rows carry the exact recorded facts at the hour, in the exile cards’ wording', () => {
  const data = validateExilesReply(reply([exile('EXL', { host: [run(0, 100, '7')], legitimacy: [run(0, 100, '4512345')],
    manpower: [run(0, 100, '12400')], cap: [run(0, 100, '24000')], leaders: [run(0, 100, '3')], read: [run(0, 100, '24')] })], { 7: [{ hour: 0, tag: 'HST' }] }));
  const [row] = exilesAt(data, 50, all).rows;
  assert.deepEqual({ ...row, key: undefined }, { key: undefined, tag: 'EXL', host: 'HST', legitimacy: '4512345', manpower: 12400, cap: 24000, leaders: 3, readHour: 24 });
  assert.equal(legitimacyText(row.legitimacy!), '45.12%');
  assert.equal(legitimacyShare(row.legitimacy!), 0.4512345);
  assert.equal(manpowerText(row), '12,400 / 24,000');
  const tip = exileTooltip(row, 50, label('Exilia'), label('Hostia'), 0);
  assert.deepEqual(tip.rows.map(item => [item.label, item.value]), [['Host', 'Hostia'], ['Legitimacy', '45.12%'], ['Exile manpower', '12,400'],
    ['Exile manpower cap', '24,000'], ['Deployed air crews', '0'], ['Army leaders recruited', '3']]);
  assert.match(tip.note!, /^Exile resources last read /);
  // Zero is a recorded value, never unknown.
  assert.equal(manpowerText({ manpower: 0, cap: 0 }), '0 / 0');
});

test('unknown values read Not recorded, sort last and the host can be missing', () => {
  const data = validateExilesReply(reply([
    exile('ONE', { host: [run(0, 100, '7')], manpower: [run(0, 100, '500')], legitimacy: [run(0, 100, '100000')] }),
    exile('TWO', { host: [run(0, 100, '8')] }),
    exile('THR', { host: [run(0, 100, '7')], manpower: [run(0, 100, '0')] }),
  ], { 7: [{ hour: 0, tag: 'HST' }] }));
  const view = exilesAt(data, 10, all);
  const two = view.rows.find(row => row.tag === 'TWO')!;
  assert.equal(two.host, null);
  assert.equal(two.manpower, null);
  assert.equal(view.unknownHosts, 1);
  assert.deepEqual(exileWarnings(data, view, []), ['1 refused exile host identity: the host has no tag at this hour']);
  assert.equal(manpowerText(two), '— / —');
  const tip = exileTooltip(two, 10, label('Two'), null, null);
  assert.deepEqual(tip.rows.map(item => item.value), ['Not recorded', 'Not recorded', 'Not recorded', 'Not recorded', 'Not recorded', 'Not recorded']);
  // While the hour detail loads, air crews are left out rather than called unrecorded.
  assert.equal(exileTooltip(two, 10, label('Two'), null, undefined).rows.some(item => item.label === 'Deployed air crews'), false);
  for (const direction of ['ascending', 'descending'] as const) {
    assert.equal(sortRows(view.rows, EXILE_VALUES.manpower, direction, keyOf).at(-1)!.tag, 'TWO');
    assert.equal(sortRows(view.rows, EXILE_VALUES.legitimacy, direction, keyOf)[0].tag, 'ONE');
  }
  assert.deepEqual(sortRows(view.rows, EXILE_VALUES.manpower, 'descending', keyOf).map(row => row.tag), ['ONE', 'THR', 'TWO']);
});

test('rows appear and leave with exile status at the hour', () => {
  const data = validateExilesReply(reply([exile('EXL', { host: [run(100, 199, '7'), run(300, 400, '8')] })],
    { 7: [{ hour: 0, tag: 'AAA' }], 8: [{ hour: 0, tag: 'BBB' }] }));
  assert.equal(exilesAt(data, 99, all).rows.length, 0);
  assert.equal(exilesAt(data, 100, all).rows[0].host, 'AAA');
  // A gap between reads is not exile.
  assert.equal(exilesAt(data, 250, all).rows.length, 0);
  assert.equal(exilesAt(data, 300, all).rows[0].host, 'BBB');
  assert.equal(exilesAt(data, 401, all).rows.length, 0);
});

test('country narrowing keeps exiles that are, or are hosted by, a chosen country, at their tags of the hour', () => {
  const data = validateExilesReply(reply([
    exile('EXA', { host: [run(0, 100, '1')] }),
    exile('EXB', { host: [run(0, 100, '2')] }),
    exile('EXC', { host: [run(0, 100, '2')] }, [{ hour: 0, tag: 'EXC' }, { hour: 50, tag: 'REN' }]),
  ], { 1: [{ hour: 0, tag: 'HOA' }], 2: [{ hour: 0, tag: 'HOB' }, { hour: 60, tag: 'HBB' }] }));
  const pick = (...tags: string[]) => (tag: string) => tags.includes(tag);
  assert.deepEqual(exilesAt(data, 10, pick('EXA')).rows.map(row => row.tag), ['EXA']);
  assert.deepEqual(exilesAt(data, 10, pick('HOB')).rows.map(row => row.tag), ['EXB', 'EXC']);
  assert.deepEqual(exilesAt(data, 70, pick('HBB')).rows.map(row => [row.tag, row.host]), [['EXB', 'HBB'], ['REN', 'HBB']]);
  assert.deepEqual(exilesAt(data, 70, pick('HOB')).rows.length, 0);
  assert.deepEqual(exilesAt(data, 70, pick('REN')).rows.map(row => row.tag), ['REN']);
});

test('the read note shows only when the supplying read is older than the shown hour', () => {
  assert.equal(readNote({ readHour: 50 }, 50), null);
  assert.equal(readNote({ readHour: null }, 50), null);
  assert.match(readNote({ readHour: 26 }, 50)!, /^Exile resources last read .+\.$/);
  const row: ExileRow = { key: 'k', tag: 'EXL', host: null, legitimacy: null, manpower: null, cap: null, leaders: null, readHour: 50 };
  assert.equal(exileTooltip(row, 50, label('Exilia'), null, null).note, undefined);
});

test('the hour detail supplies deployed air crews by exile tag and must answer the shown hour', () => {
  const detail = { branchId: '1', hour: 40, view: 'exiles', exiles: [{ tag: 'EXL', host: 'HST', legitimacy: null, manpower: null, cap: null, leaders: null, deployedAir: 1200, readHour: 24 },
    { tag: 'TWO', host: null, legitimacy: null, manpower: null, cap: null, leaders: null, deployedAir: null, readHour: null }], balances: [], current: null, unavailable: [], warnings: [] };
  const air = deployedAirByTag(validateExileDetails(detail, 40));
  assert.equal(air.get('EXL'), 1200);
  assert.equal(air.get('TWO'), null);
  assert.equal(air.has('THR'), false);
  assert.throws(() => validateExileDetails(detail, 41));
  assert.throws(() => validateExileDetails({ ...detail, exiles: [{ tag: 'EXL', deployedAir: 1.5 }] }, 40));
});

test('malformed histories are refused rather than read as no exiles', () => {
  const good = () => reply([exile('EXL', { host: [run(0, 10, '7')], legitimacy: [run(0, 10, '100')] })], { 7: [{ hour: 0, tag: 'HST' }] });
  for (const change of [
    (value: ReturnType<typeof good>) => { value.exileHistory[0].host[0].value = 'HST'; },
    (value: ReturnType<typeof good>) => { value.exileHistory[0].legitimacy[0].value = '1.5'; },
    (value: ReturnType<typeof good>) => { delete (value.exileHistory[0] as Record<string, unknown>).read; },
    (value: ReturnType<typeof good>) => { value.holders = { x: [{ hour: 0, tag: 'HST' }] }; },
    (value: ReturnType<typeof good>) => { delete (value as Record<string, unknown>).holders; },
  ]) { const value = good(); change(value); assert.throws(() => validateExilesReply(value)); }
});

test('5,000 countries with many exiles are handled without a cap', () => {
  const holders: Record<string, { hour: number; tag: string }[]> = {};
  const history = Array.from({ length: 5000 }, (_, index) => {
    holders[index] = [{ hour: 0, tag: `C${index}` }];
    return exile(`E${index}`, { host: [run(0, 100, String(index))], manpower: [run(0, 100, String(index))] });
  });
  const view = exilesAt(validateExilesReply(reply(history, holders)), 50, all);
  assert.equal(view.rows.length, 5000);
  assert.equal(new Set(view.rows.map(row => row.key)).size, 5000);
  assert.equal(sortRows(view.rows, EXILE_VALUES.manpower, 'descending', keyOf)[0].tag, 'E4999');
});
