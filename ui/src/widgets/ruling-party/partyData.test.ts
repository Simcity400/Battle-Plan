import test from 'node:test';
import assert from 'node:assert/strict';
import { bandKeys, bands, leaderAt, leaderShown, ownedPartAt, partyBadges, partyNameAt, rulingAt, validatePartyHistoryReply } from './partyData.ts';
import type { PartyHistoryReply } from '../../generated/dashboard';

const REPLY: PartyHistoryReply = {
  branchId: '1', tag: 'GER', asked: 'GER', decimals: 5, warnings: [],
  unobserved: [{ firstHour: 103, lastHour: 103 }],
  ideologies: [{ token: 'democratic', name: 'Democratic', color: [0, 0, 255] }, { token: 'fascism', name: 'Fascist', color: [150, 75, 0] }],
  parties: [
    { token: 'democratic', parts: [{ part: 0, firstHour: 100, lastHour: 102 }, { part: 4, firstHour: 104, lastHour: 110 }] },
    { token: 'fascism', parts: [{ part: 1, firstHour: 100, lastHour: 110 }] },
    { token: 'mod_ideology', parts: [{ part: 0, firstHour: 104, lastHour: 110 }] },
  ],
  ruling: [{ token: 'fascism', firstHour: 100, lastHour: 102, startsSegment: true }, { token: 'democratic', firstHour: 104, lastHour: 106, startsSegment: true },
    { token: 'fascism', firstHour: 107, lastHour: 110, startsSegment: false }],
  elections: [{ hour: 104, witnessed: 104 }, { hour: 108, witnessed: 109 }],
  governmentChanges: [{ hour: 107, witnessed: 107, before: 'democratic', after: 'fascism', cause: 'coup' }, { hour: 108, witnessed: 110, before: null, after: 'fascism', cause: null }],
  governmentUnobserved: [{ firstHour: 103, lastHour: 103 }],
  leaders: [], people: [], ideologyTypes: [],
  partyNames: ['NSDAP', null], partyNameRuns: [{ name: 0, firstHour: 100, lastHour: 102 }, { name: 1, firstHour: 104, lastHour: 106 }],
};

test('bands keep the product order and take their colours; unknown ideologies use the palette', () => {
  const list = bands(REPLY, ['#111111', '#222222']);
  assert.deepEqual(list.map(band => [band.token, band.name]), [
    ['democratic', 'Democratic'], ['fascism', 'Fascist'], ['mod_ideology', 'Ideology name unavailable'],
  ]);
  assert.equal(list[2]!.color, '#111111');
  assert.deepEqual(bandKeys(list, 'slot:7'), [0, 1, 4].map(part => ({ tag: 'slot:7', part: String(part) })), 'each part is read once, by the selected slot');
  assert.deepEqual([ownedPartAt(list[0]!, 101), ownedPartAt(list[0]!, 105), ownedPartAt(list[2]!, 101), ownedPartAt(list[2]!, 105)], [0, 4, null, 0],
    'a reused part belongs to each party only over its own hours');
});

test('changes of government are the recorded ones, at their hour once read, with a cause only when recorded', () => {
  const badges = partyBadges(REPLY, token => token, 108);
  assert.deepEqual(badges.map(badge => [badge.hour, badge.glyph]), [[104, '✓'], [107, '⇆']], 'an election read at 109 and a change read at 110 are not shown at 108');
  assert.deepEqual(partyBadges(REPLY, token => token, 110).map(badge => [badge.hour, badge.glyph]), [[104, '✓'], [107, '⇆'], [108, '⇆'], [108, '✓']],
    'an election and a change show at their recorded hours once read');
  assert.deepEqual([badges[1]!.label, badges[1]!.detail], ['fascism took power from democratic', 'Coup']);
  const unnamed = partyBadges(REPLY, token => token, 110)[2]!;
  assert.deepEqual([unnamed.label, unnamed.detail], ['The government changed', undefined], 'an unrecorded side or cause is not guessed');
  assert.deepEqual(partyBadges({ ...REPLY, governmentChanges: [] }, token => token, 110).map(badge => badge.glyph), ['✓', '✓'],
    'a change of ruling party read without a recorded change of government is no badge');
  assert.equal(rulingAt(REPLY, 105), 'democratic');
  assert.equal(rulingAt(REPLY, 103), null);
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, parties: [{ token: 'x', parts: [{ part: -1, firstHour: 100, lastHour: 100 }] }] }));
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, elections: [{ hour: 105, witnessed: 104 }] }));
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, governmentChanges: [{ ...REPLY.governmentChanges[0]!, witnessed: 106 }] }));
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, governmentChanges: [{ ...REPLY.governmentChanges[0]!, cause: 'unheard' as never }] }));
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
  assert.deepEqual(partyBadges(successions, token => token, 200, true).filter(badge => badge.glyph === '♔').map(badge => [badge.hour, badge.label]),
    [[107, 'The leader changed']]);
  assert.equal(partyBadges(successions, token => token, 200).filter(badge => badge.glyph === '♔').length, 0);
  const named = { ...successions, leaders: [...successions.leaders.slice(0, 2), { ...successions.leaders[5]!, firstHour: 103, lastHour: 103 }] };
  assert.deepEqual(partyBadges(named, token => token, 200, true).filter(badge => badge.glyph === '♔').map(badge => badge.label), ['Karl Dönitz succeeded Der Führer as leader']);
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
