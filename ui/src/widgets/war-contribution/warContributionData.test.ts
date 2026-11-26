import assert from 'node:assert/strict';
import test from 'node:test';
import type { WarContributionGroup, WarMeasureSide, WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { warContributions } from '../wars/warsData.ts';
import { chosenWar, readHourFor, warChipAt, warChoicesAt } from './warContributionData.ts';
import { heldAt } from '../army-size/headHour.ts';

const observation = (first: number, last: number, viewpointSide: string[], opposingSide: string[], name = 'Synthetic war'): WarObservation => ({
  firstHour: first, lastHour: last, viewpointTag: viewpointSide[0], viewpointSide, opposingSide, attackersAreViewpoint: true, representative: null,
  name: { text: name, known: true, reason: null, key: null },
} as WarObservation);
const war = (id: string, first: number, last: number, sides: [string[], string[]]): WarTimeline => ({
  id, firstHour: first, lastHour: last, startedBy: false, endedBy: false, observations: [observation(first, last, ...sides)],
  participants: [], capitulations: [],
} as unknown as WarTimeline);

test('the saved war wins while it exists, else the war with most participants going on at the hour', () => {
  const reply = { wars: [war('1', 0, 100, [['A'], ['B']]), war('2', 10, 500, [['A', 'C'], ['B', 'D', 'E']]), war('3', 600, 900, [['A'], ['F']])] } as unknown as WarsReply;
  assert.equal(chosenWar(reply, null, 50)?.id, '2');
  assert.equal(chosenWar(reply, '1', 50)?.id, '1');
  assert.equal(chosenWar(reply, 'gone', 700)?.id, '3', 'a war going on beats a bigger war that ended');
  assert.equal(chosenWar(reply, null, 550)?.id, '2', 'between wars, the largest one fought so far');
  assert.equal(chosenWar(reply, null, -1), null, 'no war has started');
  const tied = { ...reply, wars: [reply.wars[0]!, war('2', 10, 500, [['A'], ['D']])] };
  assert.equal(chosenWar(tied, null, 50)?.id, '1', 'a later end cannot break a tie between active wars');
  const renamed = { ...reply, wars: [{ ...reply.wars[0]!, observations: [
    observation(0, 49, ['A'], ['B'], 'First name'), observation(50, 100, ['A'], ['B'], 'Later name'),
  ] }, reply.wars[2]!] };
  assert.deepEqual(warChoicesAt(renamed, 49), [{ id: '1', firstHour: 0, name: 'First name' }]);
  assert.deepEqual(warChoicesAt(renamed, 49, '3')[1], { id: '3', firstHour: null, name: 'The selected war has not started at this date.' });
  assert.equal(warChoicesAt(renamed, 50)[0]?.name, 'Later name');
  assert.equal(readHourFor(reply.wars[0], 550), 100, 'an ended war reads its last observed hour');
  const sides = reply.wars[1].observations[0];
  assert.equal(warContributions([{ viewpointSide: ['A'], opposingSide: ['B'], participants: [], measures: [], reason: null },
    { viewpointSide: ['A', 'C'], opposingSide: ['B', 'D', 'E'], participants: [], measures: [], reason: null }], sides)?.viewpointSide.length, 2);
  assert.equal(warContributions([], sides), null);
  const joined = observation(0, 100, ['A'], ['B', 'C']);
  const groups = [{ viewpointSide: ['A'], opposingSide: ['B'], participants: [], measures: [], reason: null },
    { viewpointSide: ['A'], opposingSide: ['C', 'D'], participants: [], measures: [], reason: null },
    { viewpointSide: ['A', 'C'], opposingSide: ['B'], participants: [], measures: [], reason: null }];
  assert.equal(warContributions(groups, joined), groups[0]);
  const changing = { ...reply, wars: [{ ...reply.wars[0]!, observations: [
    observation(0, 49, ['A'], ['B']), { ...observation(50, 100, ['A'], ['B']), attackersAreViewpoint: false },
  ] }] };
  const before = heldAt(null, warChipAt(changing, '1', 49).key, 49);
  const after = heldAt(before, warChipAt(changing, '1', 50).key, 50);
  assert.equal(warChipAt(changing, '1', after.hour).observation?.attackersAreViewpoint, false, 'the chip changes sides while the same war keeps playing');
});

const knownValueOf = (value: number) => ({ kind: 'known' as const, value });

test('the Sides view draws the product’s shares in two panes: share of the war on top, then each side’s countries in the product’s order', async () => {
  const { warContributionSidesProjection } = await import('./sidesProjection.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  const member = (tag: string, value: string, shareOfSide: number | null, partial = false) => ({ tag, value, shareOfSide, shareOfWar: null, partial });
  const side = (total: string, shareOfWar: number | null, members: WarMeasureSide['members'], partial = false): WarMeasureSide => ({ total, shareOfWar, partial, members });
  const country = (tag: string, side: 'viewpoint' | 'opponent') => ({ tag, side, equipmentDamage: '0', occupation: '0', airDamage: '0', strategicBombing: '0', shipDamage: '0', convoyDamage: '0', casualties: '0', unknownCasualties: '0', lendLeaseSent: '0', lendLeaseReceived: '0', score: '0', participation: '0' });
  const group: WarContributionGroup = { viewpointSide: ['BBB'], opposingSide: ['AAA', 'CCC'], reason: null,
    participants: [country('BBB', 'viewpoint'), country('AAA', 'opponent'), country('CCC', 'opponent')],
    measures: [
      { key: 'participation', viewpoint: side('100000', null, [member('BBB', '100000', 10000)]), opposing: side('100000', null, [member('CCC', '70000', 7000), member('AAA', '30000', 3000)]) },
      { key: 'casualties', viewpoint: side('500000', 4100, [member('BBB', '500000', 10000)]), opposing: side('700000', 5900, [member('CCC', '700000', 10000, true), member('AAA', '0', 0)], true) },
    ] };
  const war = { id: '1', firstHour: 10, lastHour: 500, startedBy: false, endedBy: false, participants: [], capitulations: [],
    observations: [{ ...observation(10, 500, ['BBB'], ['AAA', 'CCC']), attackersAreViewpoint: false }] } as unknown as WarTimeline;
  const wars = { available: true, wars: [war] } as unknown as WarsReply;
  const reply = { branchId: '1', hour: 100, available: true, contributions: [group], warnings: [] };
  const model = warContributionSidesProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['live', reply]]) }), { wars, saved: null });
  assert.deepEqual(model.panes?.map(pane => pane.key), ['opposing', 'viewpoint'], 'the attackers are the left pane');
  const [left, right] = model.panes!;
  assert.deepEqual(left!.rows.map(row => row.key), ['opposing:CCC', 'opposing:AAA'], 'members in the product’s order');
  assert.deepEqual(left!.totalRow!.cells.casualties!.value, { kind: 'partial', value: 0.59, reason: 'Part of this side’s casualties contribution is not attributed to a country' });
  assert.equal(left!.totalRow!.cells.participation!.value.kind, 'notApplicable', 'participation is already a share of the side');
  assert.equal(left!.rows[1]!.cells.casualties!.value.kind, 'zero', 'a part the product says is zero reads 0');
  assert.deepEqual(left!.rows[1]!.cells.casualties!.format, { decimals: 0 }, 'a zero part reads 0, not 0%');
  assert.equal(left!.rows[0]!.cells.casualties!.value.kind, 'partial');
  assert.deepEqual(left!.rows[0]!.cells.participation!.value, { kind: 'known', value: 0.7 });
  assert.equal(left!.rows[0]!.cells.participation!.format, 'share', 'every share follows the one share format');
  const { formatValueState } = await import('../../families/valueFormat.ts');
  assert.equal((formatValueState(knownValueOf(0.0042), 'share') as { text: string }).text, '0.4%', 'a small non-zero share keeps its decimal, never 0%');
  assert.equal(right!.rows[0]!.group?.label, 'Share of the side');
  assert.deepEqual(model.columns.map(column => column.key), ['country', 'participation', 'casualties'], 'only the measures the product answers are columns');
  const none = warContributionSidesProjection.project(createBoundedReadView({ hour: 5, panels: new Map() }), { wars, saved: null });
  assert.equal(none.empty, 'Choose a war to see who fought it.');
  const future = warContributionSidesProjection.project(createBoundedReadView({ hour: 5, panels: new Map() }), { wars, saved: '1' });
  assert.equal(future.empty, 'The selected war has not started at this date.');
  const ended = warContributionSidesProjection.project(createBoundedReadView({ hour: 900, panels: new Map([['ended', { ...reply, hour: 500 }]]) }), { wars, saved: null });
  assert.equal(ended.panes?.length, 2, 'an ended war is read at its last hour, never later');
  const { warContributionMeasureProjection } = await import('./measureProjection.ts');
  const warShares: Readonly<Record<string, number>> = { BBB: 4100, CCC: 5900, AAA: 0 };
  const withWar = { ...group, measures: group.measures.map(measure => (measure.key !== 'casualties' ? measure : {
    ...measure,
    viewpoint: { ...measure.viewpoint, members: measure.viewpoint.members.map(item => ({ ...item, shareOfWar: warShares[item.tag]! })) },
    opposing: { ...measure.opposing, members: measure.opposing.members.map(item => ({ ...item, shareOfWar: warShares[item.tag]! })) },
  })) };
  const measured = warContributionMeasureProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['live', { ...reply, contributions: [withWar] }]]) }), { wars, saved: null, measure: 'casualties' });
  const bars = measured.rows.filter(row => row.kind === 'data');
  assert.deepEqual(bars.map(row => row.key), ['opposing:CCC', 'opposing:AAA'], 'the attackers on the left, in the product’s order');
  assert.deepEqual(bars[0]!.value?.value, { kind: 'partial', value: 0.59, reason: 'Part of this side’s casualties contribution is not attributed to a country' });
  assert.equal(bars[0]!.rightSide?.key, 'viewpoint:BBB');
  assert.deepEqual(bars[0]!.rightSide?.value.value, { kind: 'known', value: 0.41 }, 'every figure is a share of all of the measure in this war');
  assert.deepEqual(bars[0]!.rightSide?.tooltip?.items, [
    { label: 'Recorded amount', value: '5' }, { label: 'Share', value: '41%' }, { label: 'Flag', missingReason: 'Loading' },
  ]);
  assert.equal(bars[1]!.value?.value.kind, 'zero');
  assert.equal(bars[1]!.rightSide, undefined, 'a shorter side leaves its half empty');
  assert.deepEqual(measured.pickList?.rows.map(row => [row.key, row.left.fraction, row.right.fraction, row.divided === true]), [['casualties', 0.59, 0.41, false], ['participation', null, null, true]],
    'the list offers the measures the product answers, each side’s part of its total, war participation set apart');
  assert.equal(measured.pickList?.selectedKey, 'casualties');
  assert.deepEqual(measured.captionSides?.map(item => item.value), ['59% partial', '41%']);
  assert.match(measured.caption ?? '', /^Casualties inflicted · Each country’s share of all casualties inflicted in this war$/);

  const casualties = group.measures[1]!;
  const missing = 'Directional participation ledger not recorded';
  const partialZero = member('AAA', '0', 0, true);
  const oneSide = { ...group, viewpointSide: ['BBB', 'BBB'], reason: missing, participants: group.participants.filter(person => person.side === 'opponent'),
    measures: [{ ...casualties, viewpoint: { total: null, partial: false, members: [], shareOfWar: null },
      opposing: { ...casualties.opposing, shareOfWar: null, members: [casualties.opposing.members[0]!, partialZero] } }] };
  const read = createBoundedReadView({ hour: 100, panels: new Map([['live', { ...reply, contributions: [oneSide] }]]) });
  const table = warContributionSidesProjection.project(read, { wars, saved: null });
  assert.deepEqual(table.panes![1]!.rows.map(row => row.key), ['viewpoint:BBB']);
  assert.deepEqual(table.panes![1]!.rows[0]!.cells.casualties!.value, { kind: 'notRecorded', reason: missing });
  const witnessed = table.panes![0]!.rows[1]!;
  assert.equal(witnessed.cells.casualties!.value.kind, 'partial');
  const amounts = typeof witnessed.tooltipExtra === 'function' ? witnessed.tooltipExtra() : witnessed.tooltipExtra;
  assert.deepEqual(amounts?.[0]?.value, { kind: 'partial', value: 0n, reason: 'Part of this side’s casualties contribution is not attributed to a country' });
  const byMeasure = warContributionMeasureProjection.project(read, { wars, saved: null, measure: 'casualties' });
  const [first, second] = byMeasure.rows.filter(row => row.kind === 'data');
  assert.deepEqual(first!.rightSide!.value.value, { kind: 'notRecorded', reason: missing });
  assert.equal(second!.rightSide, undefined);
  assert.deepEqual(second!.value!.value, { kind: 'notRecorded', reason: missing });
  assert.equal(second!.tooltip?.items?.[0]?.value, '0');
  assert.ok(second!.tooltip?.note);
  const both = { ...withWar, measures: [{ ...casualties, opposing: { ...casualties.opposing, members: [{ ...partialZero, shareOfWar: 0 }] } }] };
  const zeroShare = warContributionMeasureProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['live', { ...reply, contributions: [both] }]]) }), { wars, saved: null, measure: 'casualties' });
  const zero = zeroShare.rows[0];
  assert.equal(zero?.kind === 'data' && zero.value?.value.kind, 'partial');
});
