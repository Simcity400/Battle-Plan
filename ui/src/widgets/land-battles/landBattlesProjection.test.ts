import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/contract.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { deriveRowTooltip, deriveTableAudit, deriveTableMirror, resolveTableRows, type TableRow } from '../../families/table/model.ts';
import { LOG_TABLE_BASE } from '../../families/table/logHelpers.ts';
import { tableDefaults } from '../../families/table/settings.ts';
import { buildColumnLayouts } from '../../families/table/layout.ts';
import { LOADING_NAME } from '../../families/table/identityCell.ts';
import { formatValueState } from '../../families/valueFormat.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { LandBattlesTailReply } from '../../generated/dashboard';
import { LAND_BATTLES_COLUMNS, LOSSES_SO_FAR } from './definition.ts';
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

test('a coalition battle identifies every participant on both sides in the row and tooltip, and never exposes slot keys when a name is missing', () => {
  const names: Readonly<Record<string, string>> = {
    [countryKey(1)]: 'Alpha',
    [countryKey(2)]: 'Charlie',
    [countryKey(3)]: 'Bravo',
  };
  const pendingKey = countryKey(6);
  const identity: IdentityProvider = {
    resolve(kind, key) {
      const resolved = names[key];
      return {
        kind,
        key,
        holder: { status: 'ready', value: key },
        name: key === pendingKey
          ? { status: 'pending' }
          : resolved ? { status: 'ready', value: resolved } : { status: 'absent', reason: 'Country name unavailable' },
        colour: { status: 'absent', reason: 'none' },
        flagChoice: { status: 'absent', reason: 'none' },
        validInterval: [0, Infinity] as const,
        nextChangeAt: Infinity,
      };
    },
  };
  const head: LandBattlesTailReply = { branchId: '1', total: '5', offset: '0', missing: [], warnings: [], nextChange: null, rows: [
    { key: '1:9:10', hour: 10, exact: true, province: null, state: null,
      countries: [[1, 2], [3, 4]], countriesUnknown: [false, false], countriesHour: [10, 10],
      result: 'ongoing', casualties: [10, 20], icLost: [1, 2], lossesSoFar: true, divisions: [4, 3] },
    { key: '1:10:10', hour: 10, exact: true, province: null, state: null,
      countries: [[5], [3]], countriesUnknown: [false, true], countriesHour: [10, 10],
      result: 'ongoing', casualties: [1, 1], icLost: [0, 0], lossesSoFar: true, divisions: [1, 1] },
    { key: '1:11:10', hour: 10, exact: true, province: null, state: null,
      countries: [[1, 6], [3]], countriesUnknown: [false, false], countriesHour: [10, 10],
      result: 'ongoing', casualties: [1, 1], icLost: [0, 0], lossesSoFar: true, divisions: [1, 1] },
    { key: '1:12:10', hour: 10, exact: true, province: null, state: null,
      countries: [[1], [6]], countriesUnknown: [false, false], countriesHour: [10, 10],
      result: 'ongoing', casualties: [1, 1], icLost: [0, 0], lossesSoFar: true, divisions: [1, 1] },
    { key: '1:13:10', hour: 10, exact: true, province: null, state: null,
      countries: [[6, 4], [3]], countriesUnknown: [false, false], countriesHour: [10, 10],
      result: 'ongoing', casualties: [1, 1], icLost: [0, 0], lossesSoFar: true, divisions: [1, 1] },
  ] };
  const source = buildRowSourceFromHead('branch', head, undefined, 18)!;
  const view = createBoundedReadView({ hour: 18, identity, rows: new Map([['main', source]]) });
  const model = landBattlesProjection.project(view, { byDate: true });
  const unreadFlags = createBoundedReadView({ hour: 18, rows: new Map([['main', source]]),
    identity: { resolve: (kind, key, at) => ({ ...identity.resolve(kind, key, at), flagChoice: { status: 'ready', value: 'recorded_flag' } }) },
    art: { resolve: () => ({ status: 'failed', reason: 'flag file unreadable', retryable: true, retry: () => undefined }) as never } });
  for (const row of landBattlesProjection.project(unreadFlags, { byDate: true }).rows) {
    assert.doesNotThrow(() => structuredClone(row.meta), 'a row’s subject crosses the lane worker boundary, so it carries no art callbacks');
  }
  const [coalition, unresolved, mixedSide, mixedSides, pendingAndAbsent] = model.rows as [TableRow, TableRow, TableRow, TableRow, TableRow];
  assert.deepEqual(
    coalition.cells.country!.flags?.map(flag => [flag.tag, flag.separator === true]),
    [[countryKey(1), false], [countryKey(2), false], [countryKey(3), true], [countryKey(4), false]],
    'every coalition country appears on its side of the divider even when the battle name is missing',
  );
  assert.equal(coalition.cells.country!.value.kind, 'partial');
  assert.equal(coalition.cells.country!.value.kind === 'partial' ? coalition.cells.country!.value.value : null, 'Alpha, Charlie · Bravo');
  const tip = deriveRowTooltip({ kind: 'data', key: coalition.key, name: coalition.name, rank: 1, level: 1, parent: null, childCount: 0, isTreeGroup: false, isOpen: false, isTotal: false, row: coalition }, model.columns);
  assert.deepEqual(tip.rows.find(entry => entry.label === 'Attackers')?.rawValue, { kind: 'known', value: 'Alpha, Charlie' });
  const defenders = tip.rows.find(entry => entry.label === 'Defenders');
  assert.equal(defenders?.rawValue?.kind, 'partial');
  assert.equal(defenders?.rawValue?.kind === 'partial' ? defenders.rawValue.value : null, 'Bravo');
  assert.deepEqual(
    tip.rows.filter(entry => entry.label.endsWith(' flag')).map(entry => entry.label),
    ['Alpha flag', 'Charlie flag', 'Bravo flag', 'Defender flag'],
  );
  assert.equal(tip.rows.some(entry => entry.label.includes('\u0000')), false);
  const unresolvedTip = deriveRowTooltip({ kind: 'data', key: unresolved.key, name: unresolved.name, rank: 2, level: 1, parent: null, childCount: 0, isTreeGroup: false, isOpen: false, isTotal: false, row: unresolved }, model.columns);
  assert.equal(unresolvedTip.rows.find(entry => entry.label === 'Attackers')?.rawValue?.kind, 'notRecorded', 'an unresolved slot never prints its internal key as a known country');
  assert.equal(unresolvedTip.rows.find(entry => entry.label === 'Defenders')?.rawValue?.kind, 'partial');
  assert.deepEqual(
    unresolvedTip.rows.filter(entry => entry.label.endsWith(' flag')).map(entry => entry.label),
    ['Attacker flag', 'Bravo flag'],
  );
  assert.equal(unresolvedTip.rows.some(entry => entry.label.includes('\u0000')), false);
  const mixedTip = deriveRowTooltip({ kind: 'data', key: mixedSide.key, name: mixedSide.name, rank: 3, level: 1, parent: null, childCount: 0, isTreeGroup: false, isOpen: false, isTotal: false, row: mixedSide }, model.columns);
  assert.deepEqual(mixedTip.rows.find(entry => entry.label === 'Attackers')?.rawValue, { kind: 'partial', value: 'Alpha', reason: 'Name is loading' });
  assert.deepEqual(mixedSide.cells.country!.value, { kind: 'partial', value: 'Alpha · Bravo', reason: 'Name is loading' });
  assert.deepEqual(mixedSides.cells.country!.value, { kind: 'partial', value: `Alpha · ${LOADING_NAME}`, reason: 'Name is loading' });
  const pendingAbsentTip = deriveRowTooltip({ kind: 'data', key: pendingAndAbsent.key, name: pendingAndAbsent.name, rank: 5, level: 1, parent: null, childCount: 0, isTreeGroup: false, isOpen: false, isTotal: false, row: pendingAndAbsent }, model.columns);
  assert.deepEqual(pendingAbsentTip.rows.find(entry => entry.label === 'Attackers')?.rawValue, { kind: 'pending', reason: 'Name is loading; Country name unavailable' });
});

test('pending land battle identities stay pending through cell, mirror, hover and audit until resolved, while unrecorded names stay missing', () => {
  const head: LandBattlesTailReply = {
    branchId: '1', total: '1', offset: '0', missing: [], warnings: [], nextChange: null,
    rows: [{
      key: '1:77:10', hour: 10, exact: true, province: 42, state: 9,
      countries: [[1], [2]], countriesUnknown: [false, false], countriesHour: [10, 10],
      result: 'ongoing', casualties: [5, 8], icLost: [1, 2], lossesSoFar: true, divisions: [2, 3],
    }],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, 18)!;
  const makeIdentity = (mode: 'pending' | 'ready' | 'absent'): IdentityProvider => ({
    resolve(kind, key) {
      const name = mode === 'pending'
        ? { status: 'pending' as const }
        : mode === 'ready'
          ? { status: 'ready' as const, value: kind === 'province' ? 'Westford' : kind === 'state' ? 'Highland' : key === countryKey(1) ? 'Alpha' : 'Bravo' }
          : { status: 'absent' as const, reason: `${kind} name absent` };
      return {
        kind, key,
        holder: { status: 'ready', value: key },
        name,
        colour: { status: 'absent', reason: 'none' },
        flagChoice: { status: 'absent', reason: 'none' },
        validInterval: [0, Infinity] as const,
        nextChangeAt: Infinity,
      };
    },
  });
  const settings = tableDefaults(LAND_BATTLES_COLUMNS, LOG_TABLE_BASE);
  const project = (mode: 'pending' | 'ready' | 'absent') => {
    const view = createBoundedReadView({ hour: 18, identity: makeIdentity(mode), rows: new Map([['main', source]]) });
    const model = landBattlesProjection.project(view, { byDate: true });
    const flat = resolveTableRows(model, settings);
    const cols = buildColumnLayouts(model.columns, settings);
    const row = model.rows[0]!;
    const mirror = deriveTableMirror(flat, model.columns, false).rows[0]!;
    const tip = deriveRowTooltip(flat[0] as Extract<typeof flat[number], { kind: 'data' }>, model.columns);
    const audit = deriveTableAudit({ model, flatRows: flat, visibleColumns: cols });
    return { row, mirror, tip, audit };
  };

  const pending = project('pending');
  assert.equal(pending.row.name, LOADING_NAME);
  assert.equal(pending.row.cells.item!.value.kind, 'pending');
  assert.equal(pending.row.cells.place!.value.kind, 'pending');
  assert.equal(pending.row.cells.country!.value.kind, 'pending');
  const itemFormatted = formatValueState(pending.row.cells.item!.value);
  assert.equal(itemFormatted.kind, 'pending');
  assert.equal(itemFormatted.kind === 'pending' ? itemFormatted.label : null, 'Loading…');
  assert.match(pending.mirror.cells.find(c => c.column === 'item')!.spoken, /^Loading/);
  assert.match(pending.mirror.cells.find(c => c.column === 'place')!.spoken, /^Loading/);
  assert.equal(pending.tip.title, LOADING_NAME);
  assert.equal(pending.tip.rows.find(r => r.label === 'Province')?.valueState?.kind, 'pending');
  assert.equal(pending.tip.rows.find(r => r.label === 'Attackers')?.valueState?.kind, 'pending');
  assert.match(String(pending.audit.find(a => a.key === '1:77:10:item')!.value), /"pending"/);
  assert.doesNotMatch(String(pending.audit.find(a => a.key === '1:77:10:item')!.value), /"notRecorded"/);

  const ready = project('ready');
  assert.equal(ready.row.name, 'Battle of Westford');
  assert.deepEqual(ready.row.cells.item!.value, { kind: 'known', value: 'Battle of Westford' });
  assert.deepEqual(ready.row.cells.place!.value, { kind: 'known', value: 'Highland' });
  assert.equal(ready.mirror.cells.find(c => c.column === 'item')!.spoken, 'Battle of Westford');

  const absent = project('absent');
  assert.equal(absent.row.cells.item!.value.kind, 'notRecorded');
  assert.equal(absent.row.cells.place!.value.kind, 'notRecorded');
  assert.match(absent.mirror.cells.find(c => c.column === 'item')!.spoken, /^Not recorded:/);
  assert.equal(absent.tip.rows.find(r => r.label === 'Province')?.valueState?.kind, 'missing');
});
