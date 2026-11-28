import assert from 'node:assert/strict';
import test from 'node:test';
import type { FollowQuery } from '../../dashboard/regionTime.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildTailPageRequest, compileFollowPlan } from '../../families/broker.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/plan.ts';
import { ART_STATUS_ROW, deriveRowTooltip, type TableRow } from '../../families/table/model.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { ShipsSunkTailReply, SinkingRow } from '../../generated/dashboard';
import { SHIPS_SUNK_COLUMNS } from './definition.ts';
import { shipsSunkDeclaration, shipsSunkProjection } from './shipsSunkProjection.ts';

let battleNameRequests = 0;
const identities: IdentityProvider = {
  resolve(kind, key, atHour) {
    if (kind === 'battle') battleNameRequests++;
    return {
      kind, key, atHour,
      holder: { status: 'ready', value: key }, tag: { status: 'ready', value: key },
      name: kind === 'battle' ? { status: 'pending' } : { status: 'absent', reason: 'Name not recorded' },
      colour: { status: 'absent', reason: 'Colour not recorded' }, flagChoice: { status: 'absent', reason: 'Flag not recorded' },
      validInterval: [0, 1000], nextChangeAt: Infinity,
    };
  },
};

function project(rows: SinkingRow[], hour = 20) {
  const head: ShipsSunkTailReply = {
    branchId: '1', total: String(rows.length), offset: '0', rows,
    classes: [{ firstHour: 20, class: { token: 17, name: null, key: 'mod_corvette', icon: null } }],
    nextChange: null, missing: [], warnings: [],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, hour)!;
  const view = createBoundedReadView({ hour, rows: new Map([['main', source]]), identity: identities });
  return shipsSunkProjection.project(view, { prefix: 'branch:', selected: [] });
}

const sinking = (key: string, overrides: Partial<SinkingRow>): SinkingRow => ({
  key, hour: 10, observed: 20, name: 'Meridian', classToken: 17, recordedClass: 'Corvette', attacker: 'AAA', victim: 'BBB',
  seaZone: null, battle: { kind: 'notRecorded', reason: 'No recorded link' },
  sinkingCause: { kind: 'notRecorded', reason: 'Cause not recorded' }, ...overrides,
});

const linked = { kind: 'linked', id: 'combat:20:5', liveId: 'combat:20:5', resultId: null, firstHour: 5, resultHour: null } as const;

function tooltip(row: TableRow) {
  const found = { kind: 'data', key: row.key, name: row.name, row, rank: 0, level: 1, parent: null, isTreeGroup: false, isOpen: false, childCount: 0 } as const;
  const lines = deriveRowTooltip(found, SHIPS_SUNK_COLUMNS).rows;
  return { labels: lines.filter(line => line.lookupKey !== ART_STATUS_ROW).map(line => line.label), value: (label: string) => lines.find(line => line.label === label)?.rawValue };
}

test('ship rows name the class and keep the victim dimmed, with the missing mark for an unrecorded cause', () => {
  const model = project([sinking('0', {}), sinking('1', { attacker: null, sinkingCause: { kind: 'cause', text: 'Naval mine', assisted: false } })]);
  const mined = tooltip(model.rows[1]!);
  assert.deepEqual(model.rows[1]!.cells.country?.flags?.map(flag => flag.tag), ['BBB'], 'a named cause without a killer draws only the victim');
  assert.deepEqual([mined.labels.slice(1, 3), mined.value('Killer')?.kind], [['Killer', 'Victim'], 'notRecorded'], 'and its tooltip still says the killer is not recorded');
  const row = model.rows[0]!;
  assert.equal(model.empty, undefined);
  assert.equal(model.titleSummary, '2 sunk');
  assert.equal(model.kindFilter?.allLabel, 'All classes');
  assert.deepEqual(row.meta, { hour: 10, battle: null });
  assert.equal(row.cells.item?.inlineSubtext, 'Corvette');
  assert.deepEqual(row.cells.country?.flags?.map(flag => [flag.tag, !!flag.dimmed]), [['AAA', false], ['BBB', true]]);
  assert.equal(row.cells.cause?.value.kind, 'notRecorded');
  assert.equal(row.cells.cause?.missingFigure, 'Cause');
  assert.equal(tooltip(row).value('Cause')?.kind, 'notRecorded');
});

test('a battle row names the battle in bold with its recorded killer beside it, and only the battle when the killer is not recorded', () => {
  const title = 'Second Battle of Mod Strait';
  const rows = [
    sinking('0', { battle: linked, title, sinkingCause: { kind: 'ship', name: 'Mod Raider', classKey: 'mod_raider_hull', className: null, assisted: false } }),
    sinking('1', { battle: linked, title }),
    sinking('2', { battle: { ...linked, id: 'combat:22:6', liveId: 'combat:22:6', firstHour: 6 }, titleMissing: 'Battle province not recorded',
      sinkingCause: { kind: 'airWing', carrierName: null, carrierClassKey: null, carrierClassName: null, assisted: false } }),
  ];
  const model = project(rows);
  for (let hour = 21; hour <= 24; hour++) project(rows, hour);
  assert.equal(battleNameRequests, 0, 'playing the named rows never asks for a battle name');
  const [known, unknown, unnamed] = model.rows;
  assert.deepEqual(known!.cells.cause?.value, { kind: 'known', value: 'Second Battle of Mod Strait' });
  assert.equal(known!.cells.cause?.strong, true);
  assert.equal(known!.cells.cause?.inlineSubtext, 'by mod_raider_hull');
  const knownTip = tooltip(known!);
  assert.deepEqual(knownTip.value('Battle'), { kind: 'known', value: 'Second Battle of Mod Strait' });
  assert.deepEqual(knownTip.value('Sunk by'), { kind: 'known', value: 'Mod Raider' });
  assert.deepEqual(knownTip.value('Killer class'), { kind: 'known', value: 'mod_raider_hull' });
  assert.deepEqual(knownTip.labels, ['Date', 'Killer', 'Victim', 'Class', 'Sea zone', 'Battle', 'Sunk by', 'Killer class', 'First recorded'],
    'one line per fact in reading order: no joined country line, no repeated name, and the battle and its killer replace the Cause line');
  assert.deepEqual(knownTip.value('Victim'), { kind: 'partial', value: 'BBB', reason: 'Name not recorded' }, 'a country whose name is not recorded keeps its tag, marked partial, never as a known name');
  assert.equal(unknown!.cells.cause?.inlineSubtext, undefined);
  assert.deepEqual(tooltip(unknown!).value('Sunk by'), { kind: 'notRecorded', reason: 'Killer not recorded' });
  assert.deepEqual(unnamed!.cells.cause?.value, { kind: 'notRecorded', reason: 'Battle province not recorded' });
  assert.equal(unnamed!.cells.cause?.inlineSubtext, 'by aircraft');
  assert.equal(tooltip(unnamed!).value('Battle')?.kind, 'notRecorded');
  assert.equal(known!.cells.action?.actionLabel, 'Watch', 'a sinking in a recorded battle offers Watch on that battle');
  assert.deepEqual(known!.meta, { hour: known!.rowHour, battle: { key: linked.id, title: known!.cells.cause?.value } });
  assert.deepEqual(unnamed!.meta, { hour: unnamed!.rowHour, battle: { key: 'combat:22:6', title: unnamed!.cells.cause?.value } });
  const [alone] = project([sinking('3', {})]).rows;
  assert.equal(alone!.cells.action?.blank, true, 'a sinking with no recorded battle has nothing to watch');
  assert.deepEqual(alone!.meta, { hour: alone!.rowHour, battle: null });
});

test('played heads and exact pages both request the attached battle names', () => {
  const ctx = {
    track: { source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign' } }, axisOffset: 0 },
    branch: { id: '1' },
    widget: { time: { mode: 'follow' }, countries: { kind: 'all' } },
    countries: { all: true },
    options: {},
    following: true,
  } as unknown as FamilyPlanContext;
  const plan = shipsSunkDeclaration.sources(ctx, { following: true, staticSpan: null });
  const { subscriptions } = compileFollowPlan(plan, query => ({ query: query as FollowQuery | null, refused: null }));
  assert.equal(subscriptions[0]?.channel, 'panel');
  assert.equal(subscriptions[0]?.query.panel, 'ships-sunk-tail');
  assert.equal(subscriptions[0]?.query.rowNames, true);
  const page = buildTailPageRequest(plan.entries[0]!, { hour: 20, offset: 64, limit: 64 });
  assert.equal(page?.product, 'ships-sunk-tail');
  assert.equal(page?.params.rowNames, true);
});
