import test from 'node:test';
import assert from 'node:assert/strict';
import type { PartyHistoryReply } from '../../generated/dashboard';
import { bands, buildStack, formatPopularity, partyBadges, rulingAt, rulingFirst, stackedSegments, validatePartyHistoryReply } from './partyData.ts';

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
};

test('bands follow content order and colours; unknown ideologies use the palette after them', () => {
  const list = bands(REPLY, ['#111111', '#222222']);
  assert.deepEqual(list.map(band => [band.token, band.name, band.color]), [
    ['democratic', 'Democratic', 'rgb(0 0 255)'], ['fascism', 'Fascist', 'rgb(150 75 0)'], ['mod_ideology', 'mod_ideology', '#111111'],
  ]);
});

test('stacked segments break where any party is unrecorded', () => {
  const segments = stackedSegments(buildStack(bands(REPLY, ['#111111']), 5), 100, 110, 0);
  assert.equal(segments.length, 2);
  assert.deepEqual(segments[0].hours, [100, 101, 102]);
  // Tops accumulate in band order: 50, then +40, then +10.
  assert.deepEqual(segments[0].tops.map(tops => tops[0]), [50, 90, 100]);
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
