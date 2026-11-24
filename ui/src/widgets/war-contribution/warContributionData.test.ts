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
