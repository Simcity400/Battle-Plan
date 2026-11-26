import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/contract.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { deriveRowTooltip, type TableRow } from '../../families/table/model.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { LandBattlesTailReply } from '../../generated/dashboard';
import { LOSSES_SO_FAR } from './definition.ts';
import { landBattlesProjection } from './landBattlesProjection.ts';

test('paired figures keep the recorded side when the other side is missing, and every split shows both sides', () => {
  const battle = { hour: 10, exact: false, province: null, state: null, countries: [[], []] as [number[], number[]], countriesUnknown: [true, true] as [boolean, boolean],
    countriesHour: [10, 10] as [number, number], result: 'unknown' as const };
  const head: LandBattlesTailReply = { branchId: '1', total: '3', offset: '0', missing: [], warnings: [], nextChange: null, rows: [
    { ...battle, key: '1:3:10', casualties: [0, null], icLost: [null, 12], lossesSoFar: false, divisions: [2, 4] },
    { ...battle, key: '1:4:10', casualties: [360, 80_030], icLost: [0, 0], lossesSoFar: false, divisions: [12, 9] },
    { ...battle, key: '1:5:10', result: 'ongoing', casualties: [2_729, 0], icLost: [3.5, 0], lossesSoFar: true, divisions: [3, 2] },
  ] };
  const source = buildRowSourceFromHead('branch', head, undefined, 18)!;
  const view = createBoundedReadView({ hour: 18, rows: new Map([['main', source]]) });
  const rows = landBattlesProjection.project(view, { byDate: true }).rows as TableRow[];
  const [row, lopsided] = rows.filter(entry => !entry.live) as [TableRow, TableRow];
  const live = rows.find(entry => entry.live)!;
  for (const cell of [live.cells.casualties!, live.cells.icLost!]) {
    assert.deepEqual([cell.value, ...cell.pair!].map(x => x.kind === 'partial' && x.atLeast && x.reason === LOSSES_SO_FAR), [true, true, true],
      'a live battle shows the game’s tally so far as a lower bound, never as a plain zero');
  }
  assert.equal(row.cells.icLost!.pair![0]!.kind === 'notRecorded' && row.cells.icLost!.pair![0]!.reason.includes('build cost'), true);
  assert.deepEqual(lopsided.cells.casualties!.splitBar!.map(part => part.fraction), [0.03, 0.97]);
  assert.deepEqual(lopsided.cells.icLost!.splitBar, []);
  assert.deepEqual(row.cells.casualties!.pair!.map(x => x.kind), ['zero', 'notRecorded']);
  assert.equal(row.cells.casualties!.value.kind, 'notRecorded');
  assert.deepEqual(row.cells.icLost!.splitBar, [{ fraction: 1, fill: '', hatched: true }]);
  assert.deepEqual(row.cells.divisions!.value, { kind: 'known', value: 6 });
  assert.equal(row.cells.status!.value.kind, 'notRecorded');
  assert.equal(row.cells.action!.actionLabel, 'Watch');
});

test('a coalition battle identifies every participant on both sides in the row and tooltip', () => {
  const names: Readonly<Record<string, string>> = {
    [countryKey(1)]: 'Alpha',
    [countryKey(2)]: 'Charlie',
    [countryKey(3)]: 'Bravo',
    [countryKey(4)]: 'Delta',
  };
  const identity: IdentityProvider = {
    resolve(kind, key) {
      return {
        kind,
        key,
        holder: { status: 'ready', value: key },
        name: { status: 'ready', value: names[key] ?? key },
        colour: { status: 'absent', reason: 'none' },
        flagChoice: { status: 'absent', reason: 'none' },
        validInterval: [0, Infinity] as const,
        nextChangeAt: Infinity,
      };
    },
  };
  const head: LandBattlesTailReply = { branchId: '1', total: '1', offset: '0', missing: [], warnings: [], nextChange: null, rows: [
    { key: '1:9:10', hour: 10, exact: true, province: null, state: null,
      countries: [[1, 2], [3, 4]], countriesUnknown: [false, true], countriesHour: [10, 10],
      result: 'ongoing', casualties: [10, 20], icLost: [1, 2], lossesSoFar: true, divisions: [4, 3] },
  ] };
  const source = buildRowSourceFromHead('branch', head, undefined, 18)!;
  const view = createBoundedReadView({ hour: 18, identity, rows: new Map([['main', source]]) });
  const model = landBattlesProjection.project(view, { byDate: true });
  const row = (model.rows as TableRow[])[0]!;
  assert.deepEqual(row.cells.country!.value, { kind: 'known', value: 'Alpha, Charlie · Bravo, Delta' });
  assert.equal(row.cells.item!.inlineSubtext, '+1 attacker, +1 defender');
  const tip = deriveRowTooltip({ kind: 'data', key: row.key, name: row.name, rank: 1, level: 1, parent: null, childCount: 0, isTreeGroup: false, isOpen: false, isTotal: false, row }, model.columns);
  assert.deepEqual(tip.rows.find(entry => entry.label === 'Attackers')?.rawValue, { kind: 'known', value: 'Alpha, Charlie' });
  const defenders = tip.rows.find(entry => entry.label === 'Defenders');
  assert.equal(defenders?.rawValue?.kind, 'partial');
  assert.equal(defenders?.rawValue?.kind === 'partial' ? defenders.rawValue.value : null, 'Bravo, Delta');
});
