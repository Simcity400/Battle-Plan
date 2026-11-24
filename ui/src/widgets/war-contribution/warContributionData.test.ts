import assert from 'node:assert/strict';
import test from 'node:test';
import type { WarContributionGroup, WarMeasureSide, WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { warContributions } from '../wars/warsData.ts';
import { CATEGORIES, chosenWar, contributionRows, readHourFor, rowLengths, shareText } from './warContributionData.ts';

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
});

test('rows come from the product’s measures in category order, its shares and order untouched', () => {
  const side = (shareOfWar: number | null, members: WarMeasureSide['members']): WarMeasureSide => ({ total: '0', shareOfWar, partial: members.some(member => member.partial), members });
  const member = (tag: string, value: string, shareOfSide: number | null, partial = false) => ({ tag, value, shareOfSide, shareOfWar: null, partial });
  const group: WarContributionGroup = { viewpointSide: ['A', 'B'], opposingSide: ['C'], participants: [], reason: null, measures: [
    { key: 'participation', viewpoint: side(null, [member('B', '60000', 6000), member('A', '40000', 4000)]), opposing: side(null, [member('C', '100000', 10000)]) },
    { key: 'casualties', viewpoint: side(7500, [member('A', '300000', 10000, true)]), opposing: side(2500, [member('C', '100000', 10000)]) },
  ] };
  const rows = contributionRows(group);
  assert.deepEqual(rows.map(row => row.key), CATEGORIES.map(category => category.key).filter(key => key === 'casualties' || key === 'participation'));
  assert.deepEqual(rows.find(row => row.key === 'participation')!.left.members.map(item => item.tag), ['B', 'A'], 'the product’s order is kept');
  assert.deepEqual(rowLengths(rows.find(row => row.key === 'casualties')!, 'totals'), { left: 1, right: 2500 / 7500 });
  assert.equal(shareText(6000), '60.00%');
  assert.equal(shareText(null), '—', 'no share is a dash, never zero');
});

const knownValueOf = (value: number) => ({ kind: 'known' as const, value });

test('the Sides view draws the product’s shares in two panes: share of the war on top, then each side’s countries in the product’s order', async () => {
  const { warContributionSidesProjection } = await import('./sidesProjection.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  const member = (tag: string, value: string, shareOfSide: number | null, partial = false) => ({ tag, value, shareOfSide, shareOfWar: null, partial });
  const side = (total: string, shareOfWar: number | null, members: WarMeasureSide['members'], partial = false): WarMeasureSide => ({ total, shareOfWar, partial, members });
  const country = (tag: string, side: 'attacker' | 'defender') => ({ tag, side, equipmentDamage: '0', occupation: '0', airDamage: '0', strategicBombing: '0', shipDamage: '0', convoyDamage: '0', casualties: '0', unknownCasualties: '0', lendLeaseSent: '0', lendLeaseReceived: '0', score: '0', participation: '0' });
  const group: WarContributionGroup = { viewpointSide: ['BBB'], opposingSide: ['AAA', 'CCC'], reason: null,
    participants: [country('BBB', 'defender'), country('AAA', 'attacker'), country('CCC', 'attacker')],
    measures: [
      { key: 'participation', viewpoint: side('100000', null, [member('BBB', '100000', 10000)]), opposing: side('100000', null, [member('CCC', '70000', 7000), member('AAA', '30000', 3000)]) },
      { key: 'casualties', viewpoint: side('500000', 4100, [member('BBB', '500000', 10000)]), opposing: side('700000', 5900, [member('CCC', '700000', 10000, true), member('AAA', '0', 0)], true) },
    ] };
  const war = { id: '1', firstHour: 10, lastHour: 500, startedBy: false, endedBy: false, participants: [], capitulations: [],
    observations: [observation(10, 500, ['BBB'], ['AAA', 'CCC'])] } as unknown as WarTimeline;
  const wars = { available: true, wars: [war] } as unknown as WarsReply;
  const reply = { branchId: '1', hour: 100, available: true, contributions: [group], warnings: [] };
  const model = warContributionSidesProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['live', reply]]) }), { wars, saved: null, sort: 'participation' });
  assert.deepEqual(model.panes?.map(pane => pane.key), ['opposing', 'viewpoint'], 'the attackers are the left pane');
  const [left, right] = model.panes!;
  assert.deepEqual(left!.rows.map(row => row.key), ['opposing:CCC', 'opposing:AAA'], 'members in the product’s order');
  assert.deepEqual(left!.totalRow!.cells.casualties!.value, { kind: 'partial', value: 59, reason: 'Part of this side’s casualties contribution is not attributed to a country' });
  assert.equal(left!.totalRow!.cells.participation!.value.kind, 'notApplicable', 'participation is already a share of the side');
  assert.equal(left!.rows[1]!.cells.casualties!.value.kind, 'zero', 'a part the product says is zero reads 0');
  assert.deepEqual(left!.rows[1]!.cells.casualties!.format, { decimals: 0 }, 'a zero part reads 0, not 0%');
  assert.equal(left!.rows[0]!.cells.casualties!.value.kind, 'partial');
  assert.deepEqual(left!.rows[0]!.cells.participation!.value, { kind: 'known', value: 70 });
  assert.deepEqual(left!.rows[0]!.cells.participation!.format, { decimals: 0, maxDecimals: 1, fixedDecimals: true, percent: true }, 'the sorted column carries a decimal');
  assert.deepEqual(left!.rows[0]!.cells.casualties!.format, { decimals: 0, maxDecimals: 0, fixedDecimals: true, percent: true });
  const { formatValueState } = await import('../../families/valueFormat.ts');
  assert.equal((formatValueState(knownValueOf(69.24), left!.rows[0]!.cells.participation!.format) as { text: string }).text, '69.2%', 'a share reads as the game prints it');
  assert.equal(right!.rows[0]!.group?.label, 'Share of the side');
  assert.deepEqual(model.columns.map(column => column.key), ['country', 'participation', 'casualties'], 'only the measures the product answers are columns');
  const none = warContributionSidesProjection.project(createBoundedReadView({ hour: 5, panels: new Map() }), { wars, saved: null, sort: 'participation' });
  assert.equal(none.empty, 'Choose a war to see who fought it.');
  const ended = warContributionSidesProjection.project(createBoundedReadView({ hour: 900, panels: new Map([['ended', { ...reply, hour: 500 }]]) }), { wars, saved: null, sort: 'participation' });
  assert.equal(ended.panes?.length, 2, 'an ended war is read at its last hour, never later');
});
