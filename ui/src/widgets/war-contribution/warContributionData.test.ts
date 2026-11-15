import assert from 'node:assert/strict';
import test from 'node:test';
import type { WarContributionCountry, WarContributionGroup, WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { warContributions } from '../wars/warsData.ts';
import { CATEGORIES, chosenWar, contributionItems, contributionRows, contributionStructure, readHourFor, rowLengths, shareText, sideMagnitude } from './warContributionData.ts';

const score = (tag: string, side: WarContributionCountry['side'], values: Partial<Record<keyof WarContributionCountry, string>> = {}): WarContributionCountry => ({
  tag, side, equipmentDamage: '0', occupation: '0', airDamage: '0', strategicBombing: '0', shipDamage: '0', convoyDamage: '0',
  casualties: '0', unknownCasualties: '0', lendLeaseSent: '0', lendLeaseReceived: '0', score: '0', participation: '0', ...values as Partial<WarContributionCountry>,
});
const observation = (first: number, last: number, viewpointSide: string[], opposingSide: string[], name = 'Synthetic war'): WarObservation => ({
  firstHour: first, lastHour: last, viewpointTag: viewpointSide[0], viewpointSide, opposingSide, attackersAreViewpoint: true, representative: null,
  name: { text: name, known: true, reason: null, key: null },
} as WarObservation);
const war = (id: string, first: number, last: number, sides: [string[], string[]]): WarTimeline => ({
  id, firstHour: first, lastHour: last, startedBy: false, endedBy: false, observations: [observation(first, last, ...sides)],
  participants: [], capitulations: [],
} as unknown as WarTimeline);

test('each term sums its side once per participant, keeping every member part and unknown casualties apart', () => {
  const group: WarContributionGroup = { viewpointSide: ['opaque A', 'mod:Ω', 'opaque A'], opposingSide: ['enemy'], reason: null, participants: [
    score('opaque A', 'viewpoint', { equipmentDamage: '150000', casualties: '-200000', unknownCasualties: '50000', participation: '60000' }),
    score('mod:Ω', 'viewpoint', { equipmentDamage: '50000', participation: '40000' }),
    score('enemy', 'opponent', { equipmentDamage: '300000', participation: '100000' }),
    score('opaque A', 'viewpoint', { equipmentDamage: '999999999' }),
  ] };
  const rows = contributionRows(group);
  assert.deepEqual(rows.map(row => row.key), CATEGORIES.map(category => category.key));
  const equipment = rows[0];
  assert.equal(equipment.left.total, 200000n, 'a repeated tag is one participant, its first native score');
  assert.deepEqual(equipment.left.members.map(member => member.tag), ['opaque A', 'mod:Ω']);
  assert.equal(equipment.right.total, 300000n);
  const casualties = rows.find(row => row.key === 'casualties')!;
  assert.equal(casualties.left.total, -200000n, 'signed scores stay signed');
  assert.equal(casualties.left.unknown, 50000n);
  assert.equal(rows[0].left.unknown, null, 'only casualties carry an unknown part');
  assert.deepEqual(rowLengths(equipment, 'share'), { left: 1, right: 1 });
  assert.deepEqual(rowLengths(equipment, 'totals'), { left: 2 / 3, right: 1 }, 'totals scale each row to its larger side');
  const empty = rows.find(row => row.key === 'shipDamage')!;
  assert.deepEqual(rowLengths(empty, 'share'), { left: 0, right: 0 }, 'nothing yet draws no bar');
  const missing = contributionRows({ ...group, participants: group.participants.filter(row => row.tag !== 'enemy') });
  assert.equal(missing[0].right.members.length, 0, 'an unscored participant is not a zero');
});

test('the saved war wins while it exists, else the war with most participants going on at the hour', () => {
  const reply = { wars: [war('1', 0, 100, [['A'], ['B']]), war('2', 10, 500, [['A', 'C'], ['B', 'D', 'E']]), war('3', 600, 900, [['A'], ['F']])] } as unknown as WarsReply;
  assert.equal(chosenWar(reply, null, 50)?.id, '2');
  assert.equal(chosenWar(reply, '1', 50)?.id, '1');
  assert.equal(chosenWar(reply, 'gone', 700)?.id, '3', 'a war going on beats a bigger war that ended');
  assert.equal(chosenWar(reply, null, 550)?.id, '2', 'between wars, the largest one fought so far');
  assert.equal(chosenWar(reply, null, -1), null, 'no war has started');
  assert.equal(readHourFor(reply.wars[0], 550), 100, 'an ended war reads its last observed hour');
  const sides = reply.wars[1].observations[0];
  assert.equal(warContributions([{ viewpointSide: ['A'], opposingSide: ['B'], participants: [], reason: null },
    { viewpointSide: ['A', 'C'], opposingSide: ['B', 'D', 'E'], participants: [], reason: null }], sides)?.viewpointSide.length, 2);
  assert.equal(warContributions([], sides), null);
  // One war over several native groups: the largest within it, never their sum.
  const joined = observation(0, 100, ['A'], ['B', 'C']);
  const groups = [{ viewpointSide: ['A'], opposingSide: ['B'], participants: [], reason: null },
    { viewpointSide: ['A'], opposingSide: ['C', 'D'], participants: [], reason: null },
    { viewpointSide: ['A', 'C'], opposingSide: ['B'], participants: [], reason: null }];
  assert.equal(warContributions(groups, joined), groups[0]);
});

test('shares are exact hundredths of the side, signed, and none for an empty side', () => {
  assert.equal(shareText(1n, 3n), '33.33%');
  assert.equal(shareText(2n, 3n), '66.67%');
  assert.equal(shareText(-1n, 4n), '-25.00%');
  assert.equal(shareText(-1n, -4n), '25.00%', 'a part of a negative total shares it');
  assert.equal(shareText(1n, -4n), '-25.00%');
  assert.equal(shareText(5n, 0n), '—');
});

test('a negative score keeps its bar, drawn by magnitude and outlined, never zero', () => {
  const group: WarContributionGroup = { viewpointSide: ['A', 'B'], opposingSide: ['C'], reason: null, participants: [
    score('A', 'viewpoint', { lendLeaseReceived: '-100000' }), score('B', 'viewpoint', { lendLeaseReceived: '-300000' }),
    score('C', 'opponent', { lendLeaseReceived: '200000' }),
  ] };
  const received = contributionRows(group).find(row => row.key === 'lendLeaseReceived')!;
  assert.equal(received.left.total, -400000n);
  assert.deepEqual(rowLengths(received, 'share'), { left: 1, right: 1 });
  assert.deepEqual(rowLengths(received, 'totals'), { left: 1, right: 0.5 });
  assert.equal(shareText(received.left.members[1].value, received.left.total), '75.00%');
  const cancelling = contributionRows({ ...group, participants: [
    score('A', 'viewpoint', { lendLeaseReceived: '100000' }), score('B', 'viewpoint', { lendLeaseReceived: '-100000' }),
    score('C', 'opponent', { lendLeaseReceived: '100000' }),
  ] }).find(row => row.key === 'lendLeaseReceived')!;
  assert.equal(cancelling.left.total, 0n);
  assert.equal(sideMagnitude(cancelling.left), 200000n, 'parts that cancel out keep their length');
  assert.deepEqual(rowLengths(cancelling, 'share'), { left: 1, right: 1 });
  assert.deepEqual(rowLengths(cancelling, 'totals'), { left: 1, right: 0.5 });
});

test('the bars of a group give each member its part and hatch unknown casualties, and only its members make its structure', () => {
  const group: WarContributionGroup = { viewpointSide: ['A', 'B'], opposingSide: ['C'], reason: null, participants: [
    score('A', 'viewpoint', { equipmentDamage: '100000', casualties: '-100000', unknownCasualties: '50000' }),
    score('B', 'viewpoint', { equipmentDamage: '300000' }),
    score('C', 'opponent', { equipmentDamage: '200000' }),
  ] };
  const items = contributionItems(contributionRows(group), 'totals', { colour: tag => `colour:${tag}` }, 'muted');
  const equipment = items.find(item => item.id === 'equipmentDamage')!;
  assert.deepEqual(equipment.leftParts!.map(part => [part.id, part.value, part.color]), [['A', 1, 'colour:A'], ['B', 3, 'colour:B']]);
  assert.deepEqual([equipment.left, equipment.right], [1, 0.5]);
  const casualties = items.find(item => item.id === 'casualties')!;
  assert.deepEqual(casualties.leftParts!.map(part => [part.id, part.negative ?? false, part.hatched ?? false]), [['A', true, false], ['A:unknown', false, true], ['B', false, false]]);
  const moved = { ...group, participants: group.participants.map(row => ({ ...row, equipmentDamage: '900000' })) };
  assert.equal(contributionStructure(moved, true), contributionStructure(group, true), 'numbers moving is no new structure');
  assert.notEqual(contributionStructure({ ...group, viewpointSide: ['A'] }, true), contributionStructure(group, true));
  assert.notEqual(contributionStructure(null, true), contributionStructure(null, false));
});
