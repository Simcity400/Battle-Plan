import assert from 'node:assert/strict';
import test from 'node:test';
import { sumFlowParts } from '../../charts/flow/sum.ts';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups.ts';
import { campaignScale, emptyMessage, flowParts, holderKey, linkFilter, linkHolders, linksAt, partnersOf, requestTags, withoutRequest } from './volunteersData.ts';
import type { CountryGroupReply, MilitaryAidLink, VolunteersExpeditionariesReply as VolunteersReply } from '../../generated/dashboard';

const run = (firstHour: number, lastHour: number, divisions: number | null, startsSegment = false) => ({ firstHour, lastHour, divisions, startsSegment });
const link = (key: string, sender: string, host: string, runs: MilitaryAidLink['runs'], senderSlot = 1, hostSlot = 2): MilitaryAidLink =>
  ({ key, sender, senderSlot, senderCountryId: senderSlot * 10, host, hostSlot, hostCountryId: hostSlot * 10, runs });
const reply = { tallies: [], unobserved: [], warnings: [], links: [
  link('ab', 'A', 'B', [run(0, 9, 2), run(10, 19, 3), run(20, 29, 0)]),
  link('cb', 'C', 'B', [run(5, 29, null)], 3),
  link('od', 'mod:Ω', 'D', [run(0, 9, 1), run(15, 29, 4, true)], 4, 5),
] } as unknown as VolunteersReply;
const all = () => true;

test('links at an hour keep known positive counts, count unknown ones and date their stretch', () => {
  const at12 = linksAt(reply, 12, all);
  assert.deepEqual(at12.links, [{ key: 'ab', sender: 'A', host: 'B', senderKey: holderKey('A', 1, 10), hostKey: holderKey('B', 2, 20), divisions: 3, since: 0 }],
    'the stretch began at the first positive read');
  assert.equal(at12.unknown, 1, 'an unknown count is counted, never drawn as zero');
  assert.deepEqual(linksAt(reply, 25, all).links.map(item => [item.sender, item.divisions, item.since]), [['mod:Ω', 4, 15]], 'a zero ends the link; a segment start re-dates');
  assert.equal(linksAt(reply, 40, all).links.length, 0);
  assert.deepEqual(flowParts(at12.links).map(part => [part.source, part.target, part.value]), [[holderKey('A', 1, 10), holderKey('B', 2, 20), 3]]);
});

test('chosen countries narrow to their links, the campaign scale is the busiest hour and a node lists its partners', () => {
  const chosen = (tags: string[]) => (tag: string) => tags.includes(tag);
  assert.equal(linkFilter(chosen(['A']), false)({ sender: 'A', host: 'B' }, 0), true);
  assert.equal(linkFilter(chosen(['A']), true)({ sender: 'A', host: 'B' }, 0), false);
  assert.equal(linkFilter(chosen(['A', 'B']), true)({ sender: 'A', host: 'B' }, 0), true);
  assert.equal(linkFilter(null, true)({ sender: 'X', host: 'Y' }, 0), true);
  assert.equal(campaignScale(reply, all), 7, 'the largest total one side carries at any hour');
  const { sent, received } = partnersOf(linksAt(reply, 5, all).links, holderKey('A', 1, 10));
  assert.deepEqual([sent.length, received.length], [1, 0]);
});

type Identity = { tag: string; identities: { firstHour: number; lastHour: number; slot: number; countryId: number }[] };
const groupsReply = (humans: CountryGroupReply['humans'], countryIdentities: Identity[], unobserved: CountryGroupReply['unobserved']['humans'] = []): CountryGroupReply => ({
  branchId: '1', warnings: [], humans, factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], countryIdentities,
  unobserved: { humans: unobserved, factions: [], wars: [] } });
const steady = (tag: string, slot: number): Identity => ({ tag, identities: [{ firstHour: 0, lastHour: 200, slot, countryId: slot * 10 }] });
const humans = (membership: 'hour' | 'end' | 'ever') => ({ kind: 'group' as const, group: { type: 'humans' as const }, membership, aggregate: false });

test('a group chosen by its members at each hour counts a country only while it is a member', () => {
  const groups = groupsReply([{ tag: 'B', firstHour: 10, lastHour: 19 }], [steady('A', 1), steady('B', 2), steady('C', 3)]);
  const keep = linkFilter(countryMatch(humans('hour'), [], groups, 200).matches, false);
  const hosted = { tallies: [], unobserved: [], warnings: [], links: [
    link('ab', 'A', 'B', [run(0, 29, 5)]), link('cb', 'C', 'B', [run(0, 4, 9), run(5, 29, 1)], 3)] } as unknown as VolunteersReply;
  assert.deepEqual(linksAt(hosted, 5, keep).links, [], 'before it joins');
  assert.deepEqual(linksAt(hosted, 12, keep).links.map(item => item.key), ['ab', 'cb'], 'while it is a member');
  assert.deepEqual(linksAt(hosted, 25, keep).links, [], 'after it leaves');
  const boundaries = (countryMatchFilter(humans('hour'), [], groups, 200).membership ?? []).map(span => span.firstHour);
  assert.equal(campaignScale(hosted, keep, boundaries), 6, 'the scale is the busiest hour it is a member, not 14 from before it joined');
  assert.equal(campaignScale(hosted, linkFilter(countryMatch(humans('ever'), [], groups, 200).matches, false)), 14, 'a group of members ever counts every hour');
});

test('group membership not recorded at an hour is not an empty roster', () => {
  const groups = groupsReply([{ tag: 'B', firstHour: 0, lastHour: 200 }], [steady('A', 1), steady('B', 2)], [{ firstHour: 15, lastHour: 15 }]);
  const match = countryMatch(humans('hour'), [], groups, 200);
  const hosted = { tallies: [], unobserved: [], warnings: [], links: [link('ab', 'A', 'B', [run(0, 29, 5)])] } as unknown as VolunteersReply;
  assert.equal(linksAt(hosted, 15, linkFilter(match.matches, false)).links.length, 0, 'no link counts where membership is unknown');
  const gap = countryMatchFilter(humans('hour'), [], groups, 200).unobserved.some(span => span.firstHour <= 15 && 15 <= span.lastHour);
  assert.equal(emptyMessage('land', { aidUnobserved: false, membershipUnobserved: gap, unknown: 0 }), 'This group’s membership was not recorded at this date.');
  assert.equal(emptyMessage('land', { aidUnobserved: false, membershipUnobserved: false, unknown: 0 }), 'No volunteers abroad at this date.', 'a proven empty hour');
  assert.equal(emptyMessage('expeditionary', { aidUnobserved: true, membershipUnobserved: gap, unknown: 2 }), 'Not recorded at this date.');
});

test('a group at the end or ever follows native holders through a reused tag and a rename', () => {
  const identities: Identity[] = [
    { tag: 'R', identities: [{ firstHour: 0, lastHour: 23, slot: 1, countryId: 10 }, { firstHour: 24, lastHour: 200, slot: 2, countryId: 20 }] },
    { tag: 'Q', identities: [{ firstHour: 24, lastHour: 200, slot: 1, countryId: 10 }] }, steady('H', 9)];
  const aid = { tallies: [], unobserved: [], warnings: [], links: [link('xr', 'R', 'H', [run(0, 23, 2)], 1, 9),
    link('xq', 'Q', 'H', [run(24, 99, 3)], 1, 9), link('yr', 'R', 'H', [run(24, 99, 4)], 2, 9)] } as unknown as VolunteersReply;
  const keys = (groups: CountryGroupReply, membership: 'end' | 'ever', hour: number) =>
    linksAt(aid, hour, linkFilter(countryMatch(humans(membership), [], groups, 200).matches, false)).links.map(item => item.key);
  const atEnd = groupsReply([{ tag: 'Q', firstHour: 24, lastHour: 200 }], identities);
  assert.deepEqual([keys(atEnd, 'end', 10), keys(atEnd, 'end', 30)], [['xr'], ['xq']], 'the member under its earlier tag, and not the later holder of that tag');
  assert.deepEqual(requestTags(false, countryMatchFilter(humans('end'), [], atEnd, 200, undefined, 0).membership), ['Q', 'R'], 'the request names every tag the member carried');
  const ever = groupsReply([{ tag: 'R', firstHour: 24, lastHour: 200 }], identities);
  assert.deepEqual([keys(ever, 'ever', 10), keys(ever, 'ever', 30)], [[], ['yr']]);
  assert.deepEqual(requestTags(false, countryMatchFilter(humans('ever'), [], ever, 200, undefined, 0).membership), ['R']);
});

test('the request names only the chosen country’s branch-wide tags, however many countries the campaign has', () => {
  const identities: Identity[] = [
    { tag: 'R', identities: [{ firstHour: 0, lastHour: 23, slot: 1, countryId: 10 }, { firstHour: 24, lastHour: 200, slot: 2, countryId: 20 }] },
    { tag: 'Q', identities: [{ firstHour: 24, lastHour: 200, slot: 1, countryId: 10 }] },
    ...Array.from({ length: 5000 }, (_, index) => steady(`U${index}`, index + 100))];
  const groups = groupsReply([], identities);
  const filter = countryMatchFilter({ kind: 'tags', tags: ['R'] }, [], groups, 200, undefined, 0);
  assert.deepEqual(requestTags(false, filter.membership), ['Q', 'R'], 'both holders of a picked tag, with the later tag of the first, and none of 5,000 others');
  assert.equal(requestTags(true, null), null, 'only All asks for every link');
  assert.deepEqual(requestTags(false, null), [], 'a choice not resolved yet asks for nothing, never everything');
});

test('two countries holding one tag in turn are two nodes, never one eased into the other', () => {
  const reused = { tallies: [], unobserved: [], warnings: [], links: [
    link('old', 'A', 'B', [run(0, 23, 2)], 1), link('new', 'A', 'B', [run(24, 99, 2)], 7)] } as unknown as VolunteersReply;
  const before = sumFlowParts(flowParts(linksAt(reused, 20, all).links), 12);
  const after = sumFlowParts(flowParts(linksAt(reused, 30, all).links), 12);
  const sources = (diagram: typeof before) => diagram.nodes.filter(node => node.side === 'source').map(node => node.id);
  assert.notDeepEqual(sources(before), sources(after), 'the node of the earlier holder is not the node of the later one');
  assert.notDeepEqual(before.links.map(item => item.id), after.links.map(item => item.id));
  assert.deepEqual(linkHolders(linksAt(reused, 30, all).links), [{ key: holderKey('A', 7, 70), tag: 'A', hour: 24 }, { key: holderKey('B', 2, 20), tag: 'B', hour: 24 }],
    'each holder is read at an hour its own link shows it holding the tag');
  const drill = holderKey('A', 1, 10);
  assert.equal(partnersOf(linksAt(reused, 20, all).links, drill).sent.length, 1, 'the holder clicked');
  assert.equal(partnersOf(linksAt(reused, 30, all).links, drill).sent.length, 0, 'after the tag passes on, its drill does not switch to the new holder');
});


test('with no tag to ask for, unrecorded membership, an empty group and countries not in the campaign read differently', () => {
  assert.deepEqual(withoutRequest({ hourly: true, membershipUnobserved: true }), { kind: 'unavailable', message: 'This group’s membership was not recorded at this date.' });
  assert.deepEqual(withoutRequest({ hourly: true, membershipUnobserved: false }), { kind: 'empty', message: 'This group has no members at this date.' });
  assert.equal(withoutRequest({ hourly: false, membershipUnobserved: false }).message, 'The countries chosen are not in this campaign.');
});
