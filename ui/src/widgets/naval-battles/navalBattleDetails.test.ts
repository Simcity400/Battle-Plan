import test from 'node:test';
import assert from 'node:assert/strict';
import { navalDetailTooltip, validateNavalBattleDetails } from './navalBattleDetails.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import { ART_STATUS_ROW, deriveRowTooltip } from '../../families/table/model.ts';
import { navalBattlesProjection } from './navalBattlesProjection.ts';
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

test('the followed naval table shows the supplying read, partial sinkings, unrecorded sides and an ended battle without its winner', () => {
  const live: NavalBattleLogRow = { key: 'combat:9:10', hour: 10, province: null, countries: [[], []], countriesHour: [10, 10],
    readHour: 15, detailId: 'combat:9:10', detailHour: null, shipsSunk: [2, null], ships: [3, 0], result: 'ongoing' };
  const head: NavalBattlesTailReply = { branchId: '1', total: '2', offset: '0', missing: [], warnings: [], nextChange: null,
    rows: [live, { ...live, key: 'combat:8:9', hour: 9, detailId: 'combat:8:9', result: 'unknown' }] };
  const source = buildRowSourceFromHead('branch', head, undefined, 18)!;
  const view = createBoundedReadView({ hour: 18, rows: new Map([['main', source]]) });
  const model = (following: boolean) => navalBattlesProjection.project(view, { scope: 'test', following, byDate: true });
  const project = (following: boolean) => model(following).rows[0]!;
  const row = project(true);
  assert.match(model(true).columns.find(column => column.key === 'shipsSunk')!.partialReason!, /lower bound/);
  const ended = model(true).rows[1]!.cells.status!;
  assert.deepEqual([ended.value, ended.statusKind], [{ kind: 'known', value: 'Ended' }, 'finished']);
  assert.deepEqual(ended.tooltipRows!.map(line => [line.label, line.value.kind]), [['Result', 'known'], ['Winner', 'notRecorded']]);
  const endedRow = model(true).rows[1]!;
  const found = { kind: 'data', key: endedRow.key, name: endedRow.name, row: endedRow, rank: 0, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 } as const;
  const lines = deriveRowTooltip(found, model(true).columns).rows.filter(line => line.lookupKey !== ART_STATUS_ROW);
  assert.deepEqual(lines.map(line => [line.label, line.rawValue?.kind]), [['Date', 'known'], ['Battle', 'notRecorded'], ['Result', 'known'], ['Winner', 'notRecorded'],
    ['Attackers', 'notRecorded'], ['Ships', 'known'], ['Ships sunk', 'partial'], ['Defenders', 'notRecorded'], ['Ships', 'zero'], ['Ships sunk', 'notRecorded']],
    'the tooltip groups the figures under each side instead of repeating the pair columns');
  assert.deepEqual(row.cells.shipsSunk!.pair!.map(value => value.kind), ['partial', 'notRecorded']);
  assert.equal(row.cells.shipsSunk!.value.kind, 'notRecorded');
  assert.equal(row.cells.ships!.value.kind, 'known');
  assert.equal(row.cells.ships!.pair![1].kind, 'zero');
  assert.deepEqual(row.cells.country!.flags!.map(flag => flag.tag), ['?', '?']);
  assert.equal(row.live, true);
  assert.deepEqual(JSON.parse(row.tooltipScrollId!), ['test', 'combat:9:10', 15]);
  assert.deepEqual(JSON.parse(project(false).tooltipScrollId!), ['test', 'combat:9:10', 18]);
});
