import test from 'node:test';
import assert from 'node:assert/strict';
import type { PartyHistoryReply } from '../../generated/dashboard';
import { bands, buildStack, formatPopularity, leaderAt, leaderShown, partyBadges, partyNameAt, rulingAt, rulingFirst, stackedSegments, validatePartyHistoryReply } from './partyData.ts';

const run = (firstHour: number, lastHour: number, percent: number, startsSegment = false) => ({ firstHour, lastHour, valueRaw: String(percent * 100000), startsSegment });
const REPLY: PartyHistoryReply = {
  branchId: '1', tag: 'GER', decimals: 5, warnings: [],
  unobserved: [{ firstHour: 103, lastHour: 103 }],
  ideologies: [{ token: 'democratic', name: 'Democratic', color: [0, 0, 255] }, { token: 'fascism', name: 'Fascist', color: [150, 75, 0] }],
  parties: [
    { token: 'mod_ideology', runs: [run(100, 102, 10, true), run(104, 110, 10, true)] },
    { token: 'fascism', runs: [run(100, 101, 40, true), run(102, 102, 45), run(104, 110, 20, true)] },
    { token: 'democratic', runs: [run(100, 102, 50, true), run(104, 110, 70, true)] },
  ],
  ruling: [{ token: 'fascism', firstHour: 100, lastHour: 102, startsSegment: true }, { token: 'democratic', firstHour: 104, lastHour: 106, startsSegment: true },
    { token: 'fascism', firstHour: 107, lastHour: 110, startsSegment: false }],
  elections: [104],
  leaders: [], people: [], ideologyTypes: [],
  partyNames: ['NSDAP', null], partyNameRuns: [{ name: 0, firstHour: 100, lastHour: 102 }, { name: 1, firstHour: 104, lastHour: 106 }],
};

test('bands follow content order and colours; unknown ideologies use the palette after them', () => {
  const list = bands(REPLY, ['#111111', '#222222']);
  assert.deepEqual(list.map(band => [band.token, band.name, band.color]), [
    ['democratic', 'Democratic', 'rgb(0 0 255)'], ['fascism', 'Fascist', 'rgb(150 75 0)'], ['mod_ideology', 'Ideology name unavailable', '#111111'],
  ]);
});

test('stacked segments break where any party is unrecorded', () => {
  const segments = stackedSegments(buildStack(bands(REPLY, ['#111111']), 5), 100, 110, 0);
  assert.equal(segments.length, 2);
  assert.deepEqual(segments[0].hours, [100, 101, 102]);
  // Tops accumulate in band order: 50, then +40, then +10.
  assert.deepEqual(segments[0].bands.map(band => band.high[0]), [50, 90, 100]);
  assert.deepEqual(segments[1].hours, [104, 110]);
  // Thinning keeps each segment's last sample.
  assert.deepEqual(stackedSegments(buildStack(bands(REPLY, ['#111111']), 5), 100, 110, 50).map(segment => segment.hours), [[100, 102], [104, 110]]);
});

test('a new country identity breaks the bands, and an isolated hour still draws', () => {
  const identity: PartyHistoryReply = { ...REPLY, parties: [{ token: 'fascism', runs: [run(100, 109, 40, true), run(110, 120, 60, true)] }, { token: 'democratic', runs: [run(100, 120, 20, true)] }] };
  assert.equal(stackedSegments(buildStack(bands(identity, ['#111111']), 5), 100, 120, 0).length, 2);
  const lonely: PartyHistoryReply = { ...REPLY, parties: [{ token: 'fascism', runs: [run(100, 100, 40, true), run(102, 102, 45, true)] }] };
  const drawn = stackedSegments(buildStack(bands(lonely, ['#111111']), 5), 100, 110, 0);
  assert.deepEqual(drawn.map(segment => segment.hours), [[100, 101], [102, 103]]);
});

test('a change of government next to a gap is not a badge; elections are', () => {
  const badges = partyBadges(REPLY, token => token);
  assert.deepEqual(badges.map(badge => [badge.hour, badge.glyph]), [[104, '✓'], [107, '⇆']]);
  // A run after a new identity is not a change of government.
  const replaced = { ...REPLY, ruling: REPLY.ruling.map((item, index) => index === 2 ? { ...item, startsSegment: true } : item) };
  assert.deepEqual(partyBadges(replaced, token => token).map(badge => badge.glyph), ['✓']);
  assert.equal(badges[1].label, 'fascism took power from democratic');
  assert.equal(rulingAt(REPLY, 105), 'democratic');
  assert.equal(rulingAt(REPLY, 103), null);
  assert.equal(formatPopularity('4512345', 5), '45.1%');
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, parties: [{ token: 'x', runs: [run(5, 1, 1)] }] }));
});

test('the ruling party comes first, the others keep content order', () => {
  const band = (token: string) => ({ token, name: token, color: '#000', series: { token, runs: [] } });
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
    people: [{ character: adolf, name: 'Adolf Hitler', portrait: 'data:image/png;base64,AA', portraitMissing: null },
      { character: { ...adolf, name: 'Der Führer' }, name: 'Der Führer', portrait: null, portraitMissing: 'The game generates this portrait from inputs the recording lacks.' },
      { character: { id: 9, token: '', name: 'SCOPED' }, name: null, portrait: null, portraitMissing: 'The leader’s portraits were not recorded.' }],
    ideologyTypes: [{ token: 'nazism', name: 'Nazism' }, { token: 'mod_type', name: null }] };
  const at = (hour: number) => leaderShown(reply, leaderAt(reply, hour));
  assert.deepEqual(at(100), { name: 'Adolf Hitler', known: true, portrait: 'data:image/png;base64,AA', portraitMissing: null, ideology: 'Nazism' });
  // A rename or a new portrait is its own entry, so both follow the hour shown.
  assert.deepEqual([at(102).name, at(102).portrait], ['Der Führer', null]);
  assert.match(at(102).portraitMissing!, /generates/);
  assert.deepEqual([at(103).name, at(104).name, at(105).name], ['Leader not recorded', 'Leader not identified', 'No leader']);
  assert.equal(at(104).ideology, 'Nazism');
  // An unnamed character and an ideology content does not name say so, rather than guessing.
  assert.deepEqual([at(106).name, at(106).known, at(106).ideology], ['Leader’s name not known', false, 'Ideology name unavailable']);
  // A new character is a change of leader; a rename, a gap or an unidentified leader is not.
  const successions = { ...reply, leaders: [...reply.leaders, { ideology: 'nazism', person: 3, firstHour: 107, lastHour: 107, startsSegment: false }],
    people: [...reply.people, { character: { id: 8, token: 'GER_karl_donitz', name: 'GER_karl_donitz' }, name: 'Karl Dönitz', portrait: null, portraitMissing: null }] };
  assert.deepEqual(partyBadges(successions, token => token, true).filter(badge => badge.glyph === '♔').map(badge => [badge.hour, badge.label]),
    [[107, 'The leader changed']]);
  assert.equal(partyBadges(successions, token => token).filter(badge => badge.glyph === '♔').length, 0);
  const named = { ...successions, leaders: [...successions.leaders.slice(0, 2), { ...successions.leaders[5], firstHour: 103, lastHour: 103 }] };
  assert.deepEqual(partyBadges(named, token => token, true).filter(badge => badge.glyph === '♔').map(badge => badge.label), ['Karl Dönitz succeeded Der Führer as leader']);
  assert.throws(() => validatePartyHistoryReply({ ...reply, leaders: [...reply.leaders].reverse() }));
  assert.throws(() => validatePartyHistoryReply({ ...reply, leaders: [{ ...reply.leaders[0], person: 3 }] }));
  validatePartyHistoryReply(reply);
});

test('the ruling party is named as the game names it; unrecorded and unknown differ', () => {
  assert.equal(partyNameAt(REPLY, 101), 'NSDAP');
  assert.equal(partyNameAt(REPLY, 105), null);
  assert.equal(partyNameAt(REPLY, 103), undefined);
  assert.throws(() => validatePartyHistoryReply({ ...REPLY, partyNameRuns: [{ name: 2, firstHour: 100, lastHour: 100 }] }));
});
