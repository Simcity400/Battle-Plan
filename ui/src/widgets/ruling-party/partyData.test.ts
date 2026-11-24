import test from 'node:test';
import assert from 'node:assert/strict';
import { bands, formatPopularity, leaderAt, leaderShown, partyBadges, partyNameAt, rulingAt, rulingFirst, validatePartyHistoryReply } from './partyData.ts';
import type { PartyHistoryReply } from '../../generated/dashboard';

const REPLY: PartyHistoryReply = {
  branchId: '1', tag: 'GER', asked: 'GER', decimals: 5, warnings: [],
  unobserved: [{ firstHour: 103, lastHour: 103 }],
  ideologies: [{ token: 'democratic', name: 'Democratic', color: [0, 0, 255] }, { token: 'fascism', name: 'Fascist', color: [150, 75, 0] }],
  parties: [{ token: 'mod_ideology', parts: [3] }, { token: 'fascism', parts: [1] }, { token: 'democratic', parts: [0, 4] }],
  ruling: [{ token: 'fascism', firstHour: 100, lastHour: 102, startsSegment: true }, { token: 'democratic', firstHour: 104, lastHour: 106, startsSegment: true },
    { token: 'fascism', firstHour: 107, lastHour: 110, startsSegment: false }],
  elections: [104],
  leaders: [], people: [], ideologyTypes: [],
  partyNames: ['NSDAP', null], partyNameRuns: [{ name: 0, firstHour: 100, lastHour: 102 }, { name: 1, firstHour: 104, lastHour: 106 }],
};

test('bands follow content order and colours; unknown ideologies use the palette after them', () => {
  const list = bands(REPLY, 'GER', ['#111111', '#222222']);
  assert.deepEqual(list.map(band => [band.token, band.name]), [
    ['democratic', 'Democratic'], ['fascism', 'Fascist'], ['mod_ideology', 'Ideology name unavailable'],
  ]);
  assert.equal(list[2]!.color, '#111111');
  assert.deepEqual(list.map(band => band.keys), [[{ tag: 'GER', part: '0' }, { tag: 'GER', part: '4' }], [{ tag: 'GER', part: '1' }], [{ tag: 'GER', part: '3' }]]);
});

test('a change of government next to a gap is not a badge; elections are', () => {
  const badges = partyBadges(REPLY, token => token);
  assert.deepEqual(badges.map(badge => [badge.hour, badge.glyph]), [[104, '✓'], [107, '⇆']]);
  const replaced = { ...REPLY, ruling: REPLY.ruling.map((item, index) => index === 2 ? { ...item, startsSegment: true } : item) };
  assert.deepEqual(partyBadges(replaced, token => token).map(badge => badge.glyph), ['✓']);
  assert.equal(badges[1]!.label, 'fascism took power from democratic');
  assert.equal(rulingAt(REPLY, 105), 'democratic');
  assert.equal(rulingAt(REPLY, 103), null);
  assert.equal(formatPopularity('4512345', 5), '45.1%');
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, parties: [{ token: 'x', parts: [-1] }] }));
});

test('the ruling party comes first, the others keep content order', () => {
  const band = (token: string) => ({ token, name: token, color: '#000', keys: [] });
  const list = [band('democratic'), band('communism'), band('fascism'), band('neutrality')];
  assert.deepEqual(rulingFirst(list, 'fascism').map(item => item.token), ['fascism', 'democratic', 'communism', 'neutrality']);
  assert.deepEqual(rulingFirst(list, 'democratic').map(item => item.token), ['democratic', 'communism', 'fascism', 'neutrality']);
  assert.deepEqual(rulingFirst(list, null).map(item => item.token), ['democratic', 'communism', 'fascism', 'neutrality']);
});

test('the leader is named as recorded, and every gap in what is known is said', () => {
  const adolf = { id: 7, token: 'GER_adolf_hitler', name: 'GER_adolf_hitler' };
  const reply: PartyHistoryReply = { ...REPLY,
    leaders: [
      { ideology: 'nazism', person: 0, firstHour: 100, lastHour: 101, startsSegment: true },
      { ideology: 'nazism', person: 1, firstHour: 102, lastHour: 102, startsSegment: false },
      { ideology: 'nazism', person: null, firstHour: 104, lastHour: 104, startsSegment: true },
      { ideology: '', person: null, firstHour: 105, lastHour: 105, startsSegment: false },
      { ideology: 'mod_type', person: 2, firstHour: 106, lastHour: 106, startsSegment: false },
    ],
    people: [{ character: adolf, name: 'Adolf Hitler', portrait: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', portraitMissing: null },
      { character: { ...adolf, name: 'Der Führer' }, name: 'Der Führer', portrait: null, portraitMissing: 'The game generates this portrait from inputs the recording lacks.' },
      { character: { id: 9, token: '', name: 'SCOPED' }, name: null, portrait: null, portraitMissing: 'The leader’s portraits were not recorded.' }],
    ideologyTypes: [{ token: 'nazism', name: 'Nazism' }, { token: 'mod_type', name: null }] };
  const at = (hour: number) => leaderShown(reply, leaderAt(reply, hour));
  assert.deepEqual(at(100), { name: 'Adolf Hitler', known: true, portrait: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', portraitMissing: null, ideology: 'Nazism' });
  assert.deepEqual([at(102).name, at(102).portrait], ['Der Führer', null]);
  assert.match(at(102).portraitMissing!, /generates/);
  assert.deepEqual([at(103).name, at(104).name, at(105).name], ['Leader not recorded', 'Leader not identified', 'No leader']);
  assert.equal(at(104).ideology, 'Nazism');
  assert.deepEqual([at(106).name, at(106).known, at(106).ideology], ['Leader’s name not known', false, 'Ideology name unavailable']);
  const successions = { ...reply, leaders: [...reply.leaders, { ideology: 'nazism', person: 3, firstHour: 107, lastHour: 107, startsSegment: false }],
    people: [...reply.people, { character: { id: 8, token: 'GER_karl_donitz', name: 'GER_karl_donitz' }, name: 'Karl Dönitz', portrait: null, portraitMissing: null }] };
  assert.deepEqual(partyBadges(successions, token => token, true).filter(badge => badge.glyph === '♔').map(badge => [badge.hour, badge.label]),
    [[107, 'The leader changed']]);
  assert.equal(partyBadges(successions, token => token).filter(badge => badge.glyph === '♔').length, 0);
  const named = { ...successions, leaders: [...successions.leaders.slice(0, 2), { ...successions.leaders[5]!, firstHour: 103, lastHour: 103 }] };
  assert.deepEqual(partyBadges(named, token => token, true).filter(badge => badge.glyph === '♔').map(badge => badge.label), ['Karl Dönitz succeeded Der Führer as leader']);
  assert.throws(() => validatePartyHistoryReply({ ...reply, leaders: [...reply.leaders].reverse() }));
  assert.throws(() => validatePartyHistoryReply({ ...reply, leaders: [{ ...reply.leaders[0]!, person: 3 }] }));
  validatePartyHistoryReply(reply);
});

test('the ruling party is named as the game names it; unrecorded and unknown differ', () => {
  assert.equal(partyNameAt(REPLY, 101), 'NSDAP');
  assert.equal(partyNameAt(REPLY, 105), null);
  assert.equal(partyNameAt(REPLY, 103), undefined);
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, partyNameRuns: [{ name: 2, firstHour: 100, lastHour: 100 }] }));
});
