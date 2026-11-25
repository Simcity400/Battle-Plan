import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import { ART_STATUS_ROW, deriveRowTooltip, type TableRow } from '../../families/table/model.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { ShipsSunkTailReply, SinkingRow } from '../../generated/dashboard';
import { SHIPS_SUNK_COLUMNS } from './definition.ts';
import { shipsSunkProjection } from './shipsSunkProjection.ts';

const provinces: IdentityProvider = {
  resolve(kind, key, atHour) {
    const name = kind === 'province' && key === '601' ? 'Mod Strait' : undefined;
    return {
      kind, key, atHour,
      holder: { status: 'ready', value: key }, tag: { status: 'ready', value: key },
      name: name ? { status: 'ready', value: name } : { status: 'absent', reason: 'Name not recorded' },
      colour: { status: 'absent', reason: 'Colour not recorded' }, flagChoice: { status: 'absent', reason: 'Flag not recorded' },
      validInterval: [0, 1000], nextChangeAt: Infinity,
    };
  },
};

function project(rows: SinkingRow[]) {
  const head: ShipsSunkTailReply = {
    branchId: '1', total: String(rows.length), offset: '0', rows,
    classes: [{ firstHour: 20, class: { token: 17, name: null, key: 'mod_corvette', icon: null } }],
    nextChange: null, missing: [], warnings: [],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, 20)!;
  const view = createBoundedReadView({ hour: 20, rows: new Map([['main', source]]), identity: provinces });
  return shipsSunkProjection.project(view, { prefix: 'branch:', selected: [] });
}

const sinking = (key: string, overrides: Partial<SinkingRow>): SinkingRow => ({
  key, hour: 10, observed: 20, name: 'Meridian', classToken: 17, recordedClass: 'Corvette', attacker: 'AAA', victim: 'BBB',
  seaZone: null, battle: { kind: 'notRecorded', reason: 'No recorded link' }, battleProvince: null,
  sinkingCause: { kind: 'notRecorded', reason: 'Cause not recorded' }, ...overrides,
});

const linked = { kind: 'linked', id: 'combat:20:5', liveId: 'combat:20:5', resultId: null, firstHour: 5, resultHour: null } as const;

function tooltip(row: TableRow) {
  const found = { kind: 'data', key: row.key, name: row.name, row, rank: 0, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 } as const;
  const lines = deriveRowTooltip(found, SHIPS_SUNK_COLUMNS).rows;
  return { labels: lines.filter(line => line.lookupKey !== ART_STATUS_ROW).map(line => line.label), value: (label: string) => lines.find(line => line.label === label)?.rawValue };
}

test('ship rows name the class and keep the victim dimmed, with the missing mark for an unrecorded cause', () => {
  const model = project([sinking('0', {})]);
  const row = model.rows[0]!;
  assert.equal(model.empty, undefined);
  assert.equal(model.titleSummary, '1 sunk');
  assert.equal(model.kindFilter?.allLabel, 'All classes');
  assert.equal(row.meta, 10);
  assert.equal(row.cells.item?.inlineSubtext, 'Corvette');
  assert.deepEqual(row.cells.country?.flags?.map(flag => [flag.tag, !!flag.dimmed]), [['AAA', false], ['BBB', true]]);
  assert.equal(row.cells.cause?.value.kind, 'notRecorded');
  assert.equal(row.cells.cause?.missingFigure, 'Cause');
  assert.equal(tooltip(row).value('Cause')?.kind, 'notRecorded');
});

test('a battle row names the battle in bold with its recorded killer beside it, and only the battle when the killer is not recorded', () => {
  const model = project([
    sinking('0', { battle: linked, battleProvince: 601, sinkingCause: { kind: 'ship', name: 'Mod Raider', classKey: 'mod_raider_hull', className: null, assisted: false } }),
    sinking('1', { battle: linked, battleProvince: 601 }),
    sinking('2', { battle: linked, battleProvince: 999, sinkingCause: { kind: 'airWing', carrierName: null, carrierClassKey: null, carrierClassName: null, assisted: false } }),
  ]);
  const [known, unknown, unnamed] = model.rows;
  assert.deepEqual(known!.cells.cause?.value, { kind: 'known', value: 'Battle of Mod Strait' });
  assert.equal(known!.cells.cause?.strong, true);
  assert.equal(known!.cells.cause?.inlineSubtext, 'by mod_raider_hull');
  assert.equal(known!.cells.cause?.narrowText, 'Battle');
  const knownTip = tooltip(known!);
  assert.deepEqual(knownTip.value('Battle'), { kind: 'known', value: 'Battle of Mod Strait' });
  assert.deepEqual(knownTip.value('Sunk by'), { kind: 'known', value: 'Mod Raider' });
  assert.deepEqual(knownTip.value('Killer class'), { kind: 'known', value: 'mod_raider_hull' });
  assert.deepEqual(knownTip.labels, ['Date', 'Killer', 'Victim', 'Class', 'Sea zone', 'Battle', 'Sunk by', 'Killer class', 'First recorded'],
    'one line per fact in reading order: no joined country line, no repeated name, and the battle and its killer replace the Cause line');
  assert.deepEqual(knownTip.value('Victim'), { kind: 'known', value: 'BBB' });
  assert.equal(unknown!.cells.cause?.inlineSubtext, undefined);
  assert.deepEqual(tooltip(unknown!).value('Sunk by'), { kind: 'notRecorded', reason: 'Killer not recorded' });
  assert.deepEqual(unnamed!.cells.cause?.value, { kind: 'known', value: 'Battle' }, 'a battle whose name is not recorded still shows its killer');
  assert.equal(unnamed!.cells.cause?.inlineSubtext, 'by aircraft');
  assert.equal(tooltip(unnamed!).value('Battle')?.kind, 'notRecorded');
});
